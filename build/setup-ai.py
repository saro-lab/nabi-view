#!/usr/bin/env python3
"""Connect a private AI directory and restore ignored local agent files (Python 3.9+)."""

import argparse
import hashlib
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys


AGENT_PATHS = (
    "AGENTS.md", "AGENTS.override.md", "CLAUDE.md", "CLAUDE.local.md",
    "GEMINI.md", "QWEN.md", "CODEX.md", "COPILOT.md",
    ".aider.conf.yml", ".aider.chat.history.md", ".aider.input.history",
    ".cursorrules", ".cursorignore", ".windsurfrules", ".mcp.json",
    "opencode.json", "opencode.jsonc", ".github/copilot-instructions.md",
    ".github/instructions", ".github/prompts", ".agents", ".codex",
    ".claude", ".cursor", ".gemini", ".qwen", ".windsurf", ".opencode",
    ".roo", ".kilocode", ".continue", ".junie",
)

PRIVATE_DOCUMENT_PATHS = ("docs", "README.md", "assets/icons/README.md")


def is_link(path):
    if path.is_symlink():
        return True
    if os.name == "nt" and path.exists():
        return bool(path.lstat().st_file_attributes & stat.FILE_ATTRIBUTE_REPARSE_POINT)
    return False


def fingerprint(path):
    """Compare complete contents, including empty folders; never traverse agent links."""
    result = {}
    paths = [path] + (sorted(path.rglob("*")) if path.is_dir() else [])
    for entry in paths:
        if is_link(entry):
            raise ValueError(f"Agent files must be ordinary files/directories: {entry}")
        key = str(entry.relative_to(path))
        if entry.is_dir():
            result[key] = None
        elif entry.is_file():
            digest = hashlib.sha256()
            with entry.open("rb") as stream:
                for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(chunk)
            result[key] = digest.hexdigest()
        else:
            raise ValueError(f"Unsupported agent file: {entry}")
    return result


def copy_missing(source, destination):
    if os.path.lexists(destination):
        if fingerprint(source) != fingerprint(destination):
            raise ValueError(f"Different copies; reconcile manually: {source} / {destination}")
        return
    destination.parent.mkdir(parents=True, exist_ok=True)
    if source.is_dir():
        shutil.copytree(source, destination)
    else:
        shutil.copy2(source, destination)
    if fingerprint(source) != fingerprint(destination):
        raise OSError(f"Backup verification failed: {destination}")


def check_agent_copies(project, backup):
    found = False
    for name in AGENT_PATHS:
        local, saved = project / name, backup / name
        local_exists, saved_exists = os.path.lexists(local), os.path.lexists(saved)
        found |= local_exists or saved_exists
        local_state = fingerprint(local) if local_exists else None
        saved_state = fingerprint(saved) if saved_exists else None
        if local_exists and saved_exists and local_state != saved_state:
            raise ValueError(f"Different copies; reconcile manually: {local} / {saved}")
    if not found:
        raise ValueError("No agent files found. Prepare private agent-files/ or local AGENTS.md first.")


def create_link(link, destination):
    if os.name == "nt":
        # PowerShell literal paths avoid cmd.exe expansion of %, &, and other path characters.
        quote = lambda value: "'" + str(value).replace("'", "''") + "'"
        command = (
            "$ErrorActionPreference = 'Stop'; "
            f"New-Item -ItemType Junction -Path {quote(link)} "
            f"-Target {quote(destination)} | Out-Null"
        )
        subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", command], check=True)
    else:
        link.symlink_to(destination, target_is_directory=True)


def setup(project, destination):
    project = project.resolve()
    destination = destination.expanduser().resolve()
    link = project / "ai"
    if destination.is_relative_to(project) or project.is_relative_to(destination):
        raise ValueError("Choose an external directory that is neither inside nor a parent of the repository.")
    linked = is_link(link)
    if linked and (not link.is_dir() or link.resolve() != destination):
        raise ValueError(f"Existing ai link has a different or missing target; left unchanged: {link}")
    if os.path.lexists(link) and not link.is_dir():
        raise ValueError(f"ai is not a directory: {link}")
    if destination.exists() and not destination.is_dir():
        raise ValueError(f"Destination is not a directory: {destination}")
    migrating = link.is_dir() and not linked
    if migrating and destination.exists() and any(destination.iterdir()):
        raise ValueError("Both ai and the external directory exist. Use an empty destination; no files were merged.")

    backup = (link if migrating else destination) / "agent-files"
    check_agent_copies(project, backup)
    # Back up local settings before moving anything. Existing different copies are never overwritten.
    for name in AGENT_PATHS:
        if (project / name).exists():
            copy_missing(project / name, backup / name)

    if migrating:
        destination.parent.mkdir(parents=True, exist_ok=True)
        if destination.exists():
            destination.rmdir()  # Preflight above permits only an empty destination.
        shutil.move(str(link), str(destination))
    else:
        destination.mkdir(parents=True, exist_ok=True)

    if not linked:
        try:
            create_link(link, destination)
        except (OSError, subprocess.CalledProcessError):
            # Keep all data and restore the original layout if linking fails after migration.
            if migrating and not os.path.lexists(link):
                shutil.move(str(destination), str(link))
            raise
    if not is_link(link) or link.resolve() != destination:
        raise OSError(f"Link verification failed. Private files are at: {destination}")

    # The link is in place before restoring agent settings into the checkout.
    for name in AGENT_PATHS:
        saved = link / "agent-files" / name
        if saved.exists():
            copy_missing(saved, project / name)
    print(f"AI workspace linked: {link} -> {destination}")
    print("Agent file copies verified.")


def untrack_private_files(project):
    # Documents live in ai/docs; old public paths must also leave the index.
    result = subprocess.run(
        ["git", "ls-files", "-z", "--", "ai", *AGENT_PATHS, *PRIVATE_DOCUMENT_PATHS],
        cwd=project, check=True, stdout=subprocess.PIPE,
    )
    tracked = [os.fsdecode(name) for name in result.stdout.split(b"\0") if name]
    if tracked:
        # --cached only removes index entries; staged conflicts stop without --force.
        subprocess.run(["git", "rm", "-r", "--cached", "--", *tracked], cwd=project, check=True)
    print(f"Private paths untracked: {len(tracked)}. Local files are retained.")
    print("Past commits are unchanged; review history before publishing it.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path, help="User-selected private directory outside the repository")
    args = parser.parse_args()
    project = Path(__file__).resolve().parent.parent
    try:
        setup(project, args.directory)
        untrack_private_files(project)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Setup incomplete: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
