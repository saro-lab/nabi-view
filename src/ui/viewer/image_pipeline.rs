use super::*;

impl Viewer {
    pub(super) fn queue_images(&mut self) {
        if self.files.is_empty() {
            for slot in 0..2 {
                if self.tickets[slot].take().is_some() {
                    self.media.clear_slot(slot);
                }
            }
            self.cancel_prefetch();
            return;
        }
        let (a, b) = self.focus_span();
        if self
            .prefetch_ticket
            .as_ref()
            .is_some_and(|(_, path, _)| self.files[a..=b].contains(path))
        {
            // A current-page request must never wait behind low-priority work.
            self.cancel_prefetch();
        }
        let mate = if a == b {
            None
        } else if a == self.focus_index {
            Some(b)
        } else {
            Some(a)
        };
        for (slot, i) in [Some(self.focus_index), mate].into_iter().enumerate() {
            let Some(i) = i else {
                if self.tickets[slot].take().is_some() {
                    self.media.clear_slot(slot);
                }
                continue;
            };
            let path = &self.files[i];
            let limit = if self.settings.view_mode == "actual"
                || self.zoom_override.is_some()
                || self.zoom > 1.1
            {
                4096
            } else {
                self.fit_limit
            };
            if let Some(pic) = self.pictures.get_mut(path) {
                pic.last_used = self.frame;
                if pic.requested >= limit
                    || pic.original.iter().copied().max().unwrap_or(0) <= pic.requested
                {
                    if self.tickets[slot].as_ref().is_some_and(|t| &t.1 != path) {
                        self.media.clear_slot(slot);
                        self.tickets[slot] = None;
                    }
                    continue;
                }
            }
            if self.failures.contains_key(path)
                || self.tickets[slot].as_ref().is_some_and(|t| &t.1 == path)
            {
                continue;
            }
            let id = if slot == 0 {
                self.media.request_image(path.clone(), limit)
            } else {
                self.media.request_image_in_slot(slot, path.clone(), limit)
            };
            self.tickets[slot] = Some((id, path.clone(), limit));
        }
    }

    pub(super) fn cancel_prefetch(&mut self) {
        if self.prefetch_ticket.take().is_some() {
            self.media.clear_prefetch();
        }
    }

    pub(super) fn cache_budget(&self) -> usize {
        let configured = (u64::from(self.settings.cache_memory_mb) * 1024 * 1024)
            .min(usize::MAX as u64) as usize;
        self.memory_policy.budget(configured)
    }

    pub(super) fn apply_memory_sample(
        &mut self,
        sample: Option<crate::platform::system_memory::Snapshot>,
    ) {
        let previous = self.cache_budget();
        self.memory_policy.observe(sample, self.cache_bytes());
        if self.cache_budget() > previous {
            self.prefetch_blocked.clear();
        }
        if !self.memory_policy.background_allowed() {
            self.cancel_prefetch();
            self.media.clear_thumbnail();
            self.thumb_ticket = None;
            self.thumbs.clear();
        } else if self.cache_budget() < previous {
            // Drop pending/ready speculative pixels before collecting results.
            self.cancel_prefetch();
            self.media.clear_thumbnail();
            self.thumb_ticket = None;
        }
        self.trim_cache(0);
    }

    pub(super) fn cache_bytes(&self) -> usize {
        let mut textures = HashSet::new();
        let texture_bytes: usize = self
            .pictures
            .values()
            .map(|p| &p.handle)
            .chain(self.presentation.images.iter().map(|p| &p.handle))
            .chain(self.thumbs.values())
            .filter(|handle| textures.insert(handle.id()))
            .map(|handle| handle.byte_size())
            .fold(0usize, usize::saturating_add);
        let animation_bytes: usize = self
            .pictures
            .values()
            .flat_map(|p| &p.animation)
            .map(media::animation::Animation::retained_bytes)
            .fold(0usize, usize::saturating_add);
        texture_bytes.saturating_add(animation_bytes)
    }

    pub(super) fn nearby_cache_paths(&self) -> Vec<PathBuf> {
        if self.files.is_empty() {
            return Vec::new();
        }
        let (first, last) = self.focus_span();
        let count = self.settings.cache_image_limit.min(self.files.len());
        let mut paths = self.files[first..=last].to_vec();
        let mut seen: HashSet<_> = paths.iter().cloned().collect();
        for distance in 1..self.files.len() {
            for index in [
                (last + distance) % self.files.len(),
                (first + self.files.len() - distance) % self.files.len(),
            ] {
                if paths.len() >= count {
                    return paths;
                }
                let path = &self.files[index];
                if seen.insert(path.clone()) {
                    paths.push(path.clone());
                }
            }
        }
        paths
    }

    pub(super) fn trim_cache(&mut self, extra_bytes: usize) {
        let budget = self.cache_budget();
        let mut bytes = self.cache_bytes();
        if self.pictures.len() <= self.settings.cache_image_limit
            && bytes.saturating_add(extra_bytes) <= budget
        {
            return;
        }
        let visible = self.focus_span();
        let pinned = [self.files.get(visible.0), self.files.get(visible.1)];
        if budget == 0 {
            // Pressure can evict thousands of entries at once. Retained drawing
            // handles keep the transition image alive independently of pictures.
            self.pictures.retain(|path, _| pinned.contains(&Some(path)));
            self.thumbs.clear();
            return;
        }
        let nearby: HashSet<_> = self.nearby_cache_paths().into_iter().collect();
        let mut candidates: Vec<_> = self
            .pictures
            .iter()
            .filter(|(path, _)| !pinned.contains(&Some(*path)))
            .map(|(path, pic)| (nearby.contains(path), pic.last_used, path.clone()))
            .collect();
        candidates.sort_unstable();
        let mut references = HashMap::<_, usize>::new();
        for handle in self
            .pictures
            .values()
            .map(|p| &p.handle)
            .chain(self.presentation.images.iter().map(|p| &p.handle))
            .chain(self.thumbs.values())
        {
            *references.entry(handle.id()).or_default() += 1;
        }
        for (_, _, path) in candidates {
            if self.pictures.len() <= self.settings.cache_image_limit
                && bytes.saturating_add(extra_bytes) <= budget
            {
                break;
            }
            if let Some(picture) = self.pictures.remove(&path) {
                if let Some(frame) = &picture.animation {
                    bytes = bytes.saturating_sub(frame.retained_bytes());
                }
                let count = references.get_mut(&picture.handle.id()).unwrap();
                *count -= 1;
                if *count == 0 {
                    bytes = bytes.saturating_sub(picture.handle.byte_size());
                }
            }
        }
        // Do not cycle existing thumbnails merely to reserve another thumbnail.
        if extra_bytes == 0 && bytes > budget {
            self.thumbs.clear();
        }
    }

    pub(super) fn queue_prefetch(&mut self) {
        let settings = (
            self.settings.cache_image_limit,
            self.settings.cache_memory_mb,
            self.fit_limit,
        );
        if self.prefetch_settings != settings {
            self.prefetch_settings = settings;
            self.prefetch_blocked.clear();
            self.cancel_prefetch();
        }
        if !self.memory_policy.background_allowed()
            || self.files.is_empty()
            || self.listing.is_some()
        {
            self.cancel_prefetch();
            return;
        }
        let nearby = self.nearby_cache_paths();
        if self
            .prefetch_ticket
            .as_ref()
            .is_some_and(|(_, path, _)| !nearby.contains(path))
        {
            self.cancel_prefetch();
        }
        // Keep the working set centered on the requested pages, including wrap
        // around and the current randomized order. Never scan the whole folder.
        let desired: HashSet<_> = nearby.iter().collect();
        self.pictures.retain(|path, _| desired.contains(path));
        self.trim_cache(0);
        let (first, last) = self.focus_span();
        if self.tickets.iter().any(Option::is_some)
            || self.thumb_ticket.is_some()
            || self.files[first..=last]
                .iter()
                .any(|path| !self.pictures.contains_key(path) && !self.failures.contains_key(path))
        {
            self.cancel_prefetch();
            return;
        }
        let reserved = media::prefetch_output_budget(self.fit_limit);
        if self.pictures.len() >= self.settings.cache_image_limit
            || self.cache_bytes().saturating_add(reserved) > self.cache_budget()
        {
            self.cancel_prefetch();
            return;
        }
        if self.prefetch_ticket.is_some() {
            return;
        }
        let path = nearby.into_iter().find(|path| {
            !self.pictures.contains_key(path)
                && !self.failures.contains_key(path)
                && !self.prefetch_blocked.contains(path)
        });
        if let Some(path) = path {
            let id = self.media.request_prefetch(path.clone(), self.fit_limit);
            self.prefetch_ticket = Some((id, path, self.fit_limit));
        }
    }

    pub(super) fn refresh_folder(&mut self, mut files: Vec<PathBuf>, changed: &[PathBuf]) {
        if self.random {
            files = reconcile_random(&self.files, &files);
        }
        let focused_path = self.files.get(self.focus_index).cloned();
        let focus_index = refreshed_focus_index(&self.files, self.focus_index, &files);
        let live: HashSet<_> = files.iter().collect();
        self.checked.retain(|path| live.contains(path));
        let changed: HashSet<_> = changed.iter().collect();
        let valid = |p: &PathBuf| live.contains(p) && !changed.contains(p);
        self.pictures.retain(|p, _| valid(p));
        self.page_sizes.retain(|p, _| valid(p));
        self.prefetch_blocked.clear();
        if self.prefetch_ticket.as_ref().is_some_and(|t| !valid(&t.1)) {
            self.cancel_prefetch();
        }
        self.thumbs.retain(|p, _| valid(p));
        self.failures.retain(|p, _| valid(p));
        self.thumb_errors.retain(valid);
        for (slot, ticket) in self.tickets.iter_mut().enumerate() {
            if ticket.as_ref().is_some_and(|t| !valid(&t.1)) {
                *ticket = None;
                self.media.clear_slot(slot);
            }
        }
        if self.thumb_ticket.as_ref().is_some_and(|t| !valid(&t.1)) {
            self.thumb_ticket = None;
            self.media.clear_thumbnail();
        }
        let (first, last) = self.focus_span();
        let visible_changed = [self.files.get(first), self.files.get(last)]
            .into_iter()
            .flatten()
            .any(|p| !valid(p));
        if focused_path.as_ref() != files.get(focus_index) || visible_changed {
            self.inspector.invalidate();
        }
        if focused_path.as_ref() != files.get(focus_index) {
            self.pan = Vec2::ZERO;
            self.zoom = 1.0;
            self.zoom_override = None;
            self.zoom_input = None;
        }
        self.focus_index = focus_index;
        self.span_cache.get_mut().take();
        self.files = files;
        if self.files.is_empty() {
            self.last_error.clear();
            self.presentation = Presentation::default();
            self.transition_waiting = false;
        }
    }

    pub(super) fn collect(&mut self, ctx: &egui::Context) {
        while let Some(event) = self.media.try_recv() {
            match event {
                MediaEvent::Folder { id, path, result }
                    if self.listing == Some(id) && self.folder.as_ref() == Some(&path) =>
                {
                    self.listing = None;
                    match result {
                        Ok(files) => {
                            self.refresh_folder(files, &[]);
                            self.replay_pending_navigation(ctx);
                        }
                        Err(e) => {
                            self.last_error = format!(
                                "{e}   ·   Choose Open folder to browse neighboring images."
                            )
                        }
                    }
                }
                MediaEvent::FolderChanged { id, path, result }
                    if self.folder_watch == Some(id) && self.folder.as_ref() == Some(&path) =>
                {
                    match result {
                        Ok(update) => self.refresh_folder(update.files, &update.changed),
                        Err(error) => self.last_error = error,
                    }
                }
                MediaEvent::Image { id, path, result } => {
                    let Some(slot) = self
                        .tickets
                        .iter()
                        .position(|t| t.as_ref().is_some_and(|t| t.0 == id && t.1 == path))
                    else {
                        continue;
                    };
                    let (_, _, requested) = self.tickets[slot].take().unwrap();
                    match result {
                        Ok(frame) => {
                            let handle = ctx.load_texture(
                                format!("page-{id}"),
                                frame.image,
                                egui::TextureOptions::LINEAR,
                            );
                            self.span_cache.get_mut().take();
                            self.page_sizes.insert(path.clone(), frame.original_size);
                            self.pictures.insert(
                                path,
                                Picture {
                                    animation: frame.animation,
                                    handle,
                                    original: frame.original_size,
                                    requested,
                                    last_used: self.frame,
                                },
                            );
                        }
                        Err(e) => {
                            self.failures.insert(path, e);
                        }
                    }
                }
                MediaEvent::Thumbnail { id, path, result }
                    if self
                        .thumb_ticket
                        .as_ref()
                        .is_some_and(|t| t.0 == id && t.1 == path) =>
                {
                    self.thumb_ticket = None;
                    if !self.memory_policy.background_allowed() {
                        continue;
                    }
                    match result {
                        Ok(frame) => {
                            let bytes = frame.image.pixels.len() * 4;
                            self.trim_cache(bytes);
                            if self.cache_bytes().saturating_add(bytes) > self.cache_budget() {
                                continue;
                            }
                            // Keep every visible thumbnail even on very wide displays.
                            // The shared byte budget still bounds memory use.
                            let visible = thumbnail_range(
                                self.files.len(),
                                self.focus_index,
                                ctx.screen_rect().width() - THUMBNAIL_MARGIN * 2.,
                                ctx.style().spacing.item_spacing.x,
                            );
                            if self.thumbs.len() >= 120.max(visible.len() * 2) {
                                let keep = visible.start.saturating_sub(15)
                                    ..(visible.end + 15).min(self.files.len());
                                let near: HashSet<_> = self.files[keep].iter().collect();
                                self.thumbs.retain(|p, _| near.contains(p));
                            }
                            self.thumbs.insert(
                                path,
                                ctx.load_texture(
                                    format!("thumb-{id}"),
                                    frame.image,
                                    egui::TextureOptions::LINEAR,
                                ),
                            );
                        }
                        Err(_) => {
                            self.thumb_errors.insert(path);
                        }
                    }
                }
                MediaEvent::Prefetched { id, path, result }
                    if self
                        .prefetch_ticket
                        .as_ref()
                        .is_some_and(|t| t.0 == id && t.1 == path) =>
                {
                    let (_, _, requested) = self.prefetch_ticket.take().unwrap();
                    if !self.memory_policy.background_allowed()
                        || self.pictures.contains_key(&path)
                        || !self.nearby_cache_paths().contains(&path)
                        || requested != self.fit_limit
                    {
                        continue;
                    }
                    match result {
                        Ok(frame) => {
                            let bytes = frame.image.pixels.len() * 4;
                            if self.pictures.len() >= self.settings.cache_image_limit
                                || self.cache_bytes().saturating_add(bytes) > self.cache_budget()
                            {
                                self.prefetch_blocked.insert(path);
                                continue;
                            }
                            self.span_cache.get_mut().take();
                            self.page_sizes.insert(path.clone(), frame.original_size);
                            self.pictures.insert(
                                path,
                                Picture {
                                    animation: frame.animation,
                                    handle: ctx.load_texture(
                                        format!("prefetch-{id}"),
                                        frame.image,
                                        egui::TextureOptions::LINEAR,
                                    ),
                                    original: frame.original_size,
                                    requested,
                                    last_used: self.frame,
                                },
                            );
                        }
                        Err(_) => {
                            // Retry through the visible path only; a corrupt
                            // neighbor must not create an endless background loop.
                            self.prefetch_blocked.insert(path);
                        }
                    }
                }
                _ => {}
            }
        }
        self.queue_images();
        self.trim_cache(0);
    }
}
