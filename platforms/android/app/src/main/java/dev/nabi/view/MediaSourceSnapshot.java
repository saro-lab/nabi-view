package dev.nabi.view;

import java.util.Collections;
import java.util.Map;
import java.util.Objects;
import java.util.TreeMap;

/** An in-memory token for one imported MediaStore source, never an access grant. */
final class MediaSourceSnapshot {
    static final class Volume {
        final String version;
        final long generation;
        Volume(String version, long generation) {
            if (version == null || generation < 0) throw new IllegalArgumentException("Unavailable media volume");
            this.version = version;
            this.generation = generation;
        }
        @Override public boolean equals(Object other) {
            if (!(other instanceof Volume)) return false;
            Volume value = (Volume)other;
            return version.equals(value.version) && generation == value.generation;
        }
        @Override public int hashCode() { return Objects.hash(version, generation); }
    }

    final String source;
    // null means all currently mounted external volumes, not a synthetic generation.
    final String volume;
    final long token, epoch;
    private final Map<String, Volume> volumes;

    MediaSourceSnapshot(String source, String volume, long token, long epoch, Map<String, Volume> volumes) {
        if (token <= 0 || epoch <= 0 || volumes.isEmpty()) throw new IllegalArgumentException("Invalid media snapshot");
        this.source = source;
        this.volume = volume;
        this.token = token;
        this.epoch = epoch;
        this.volumes = Collections.unmodifiableMap(new TreeMap<>(volumes));
    }

    boolean matches(String source, long token, long epoch, boolean fullAccess, Map<String, Volume> volumes) {
        return this.source.equals(source) && this.token == token && matchesContent(epoch, fullAccess, volumes);
    }

    boolean matchesContent(long epoch, boolean fullAccess, Map<String, Volume> volumes) {
        return fullAccess && this.epoch == epoch && this.volumes.equals(volumes);
    }
}
