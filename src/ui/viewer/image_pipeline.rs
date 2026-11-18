use super::*;

impl Viewer {
    #[cfg(not(any(target_os = "ios", target_os = "android", test)))]
    pub(super) fn source_list_pending(&self) -> bool {
        false
    }

    /// Includes both submitted work and a page whose request has not been queued yet.
    pub(super) fn current_images_pending(&self) -> bool {
        if self.tickets.iter().any(Option::is_some) {
            return true;
        }
        if self.files.is_empty() {
            return false;
        }
        let (first, last) = self.focus_span();
        self.files[first..=last]
            .iter()
            .any(|path| !self.pictures.contains_key(path) && !self.failures.contains_key(path))
    }

    pub(super) fn queue_images(&mut self) {
        #[cfg(any(target_os = "ios", test))]
        if self.ios_photos_refreshing() {
            return;
        }
        if self.renaming() {
            return;
        }
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
                MAX_DECODE_SIDE
            } else {
                self.fit_limit
            };
            if let Some(pic) = self.pictures.get_mut(path) {
                pic.last_used = self.frame;
                if pic.requested >= limit
                    || pic.original.iter().copied().max().unwrap_or(0) <= pic.requested
                    // Detail crops improve a manual/actual-size view without
                    // decoding a second full raster beside the crop worker.
                    || (pic.animation.is_none() && (self.zoom_override.is_some()
                        || self.settings.view_mode == "actual" || self.zoom > 1.1))
                {
                    if self.tickets[slot].as_ref().is_some_and(|t| &t.1 != path) {
                        self.media.clear_slot(slot);
                        self.tickets[slot] = None;
                    }
                    continue;
                }
            }
            if self.failures.contains_key(path) {
                if self.tickets[slot].take().is_some() {
                    self.media.clear_slot(slot);
                }
                continue;
            }
            if self.tickets[slot].as_ref().is_some_and(|t| &t.1 == path) {
                continue;
            }
            let id = self.media.request_image_in_slot(slot, path.clone(), limit);
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
        if self.cache_budget() == 0 {
            self.details.clear();
        }
        if !self.memory_policy.background_allowed() {
            self.held_prefetch.clear();
            self.cancel_prefetch();
            self.media.clear_thumbnail();
            self.thumb_ticket = None;
            self.thumbs.clear();
            self.library.clear_thumbnails();
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
            .chain(self.details.textures())
            .chain(self.library.thumbnail_textures())
            .filter(|handle| textures.insert(handle.id()))
            .map(|handle| handle.byte_size())
            .fold(0usize, usize::saturating_add);
        let animation_bytes: usize = self
            .pictures
            .values()
            .flat_map(|p| &p.animation)
            .map(media::animation::Animation::retained_bytes)
            .fold(0usize, usize::saturating_add);
        let mut hdr_ids = HashSet::new();
        let hdr_bytes = self
            .pictures
            .values()
            .filter_map(|p| p.hdr.as_ref())
            .chain(
                self.presentation
                    .images
                    .iter()
                    .filter_map(|p| p.hdr.as_ref()),
            )
            .chain(self.details.hdr_textures())
            .filter(|hdr| hdr_ids.insert(hdr.id()))
            .map(|hdr| hdr.bytes())
            .fold(0usize, usize::saturating_add);
        texture_bytes
            .saturating_add(animation_bytes)
            .saturating_add(hdr_bytes)
    }

    /// Speculative display work around the current pages, most urgent first.
    pub(super) fn prefetch_targets(&self) -> Vec<prewarm::Target> {
        if self.files.is_empty() {
            return Vec::new();
        }
        let (first, last) = self.focus_span();
        prewarm::display_targets(prewarm::Position {
            len: self.files.len(),
            first,
            last,
            backward: self.backward,
            mobile: crate::app::is_mobile(),
            limit: self.settings.cache_image_limit.min(self.files.len()),
        })
    }

    /// Pages whose decoded images stay in memory: the current pages, then
    /// the memory part of the working set in priority order.
    pub(super) fn nearby_cache_paths(&self) -> Vec<PathBuf> {
        if self.files.is_empty() {
            return Vec::new();
        }
        let (first, last) = self.focus_span();
        self.files[first..=last]
            .iter()
            .cloned()
            .chain(
                self.prefetch_targets()
                    .into_iter()
                    .filter(|target| !target.disk_only)
                    .map(|target| self.files[target.index].clone()),
            )
            .collect()
    }

    /// Mobile range thumbnails are still being prepared.
    fn range_thumbnails_pending(&self) -> bool {
        (self.thumb_ticket.is_some() && self.thumb_urgency == media::Urgency::RangeThumbnail)
            || self.warm_thumbnail_candidate().is_some()
    }

    pub(super) fn warm_thumbnail_candidate(&self) -> Option<PathBuf> {
        if !crate::app::is_mobile() || self.source_list_pending() {
            return None;
        }
        prewarm::thumbnails(self.files.len(), self.focus_index, self.backward)
            .into_iter()
            .map(|index| &self.files[index])
            .find(|path| {
                !self.thumbs.contains_key(*path)
                    && !self.pictures.contains_key(*path)
                    && !self.thumb_errors.contains(*path)
                    && {
                        #[cfg(any(target_os = "ios", target_os = "android", test))]
                        {
                            crate::core::documents::can_warm_thumbnail(path)
                        }
                        #[cfg(not(any(target_os = "ios", target_os = "android", test)))]
                        {
                            true
                        }
                    }
            })
            .cloned()
    }

    pub(super) fn trim_cache(&mut self, extra_bytes: usize) {
        self.trim_cache_with_library(extra_bytes, true);
    }

    /// Visible library rows follow current images and precede adjacent images.
    /// The caller has already reclaimed its offscreen list thumbnails.
    pub(super) fn trim_cache_for_library(&mut self, extra_bytes: usize) {
        self.trim_cache_with_library(extra_bytes, false);
    }

    fn trim_cache_with_library(&mut self, extra_bytes: usize, trim_library: bool) {
        let budget = self.cache_budget();
        if budget == 0 {
            self.library.clear_thumbnails();
        }
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
        let excess = bytes.saturating_add(extra_bytes).saturating_sub(budget);
        if excess > 0 && trim_library {
            // Reusable list previews yield to the currently displayed images.
            self.library.trim_thumbnails(excess);
            bytes = self.cache_bytes();
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
        let mut hdr_references = HashMap::<_, usize>::new();
        for hdr in self
            .pictures
            .values()
            .filter_map(|p| p.hdr.as_ref())
            .chain(
                self.presentation
                    .images
                    .iter()
                    .filter_map(|p| p.hdr.as_ref()),
            )
            .chain(self.details.hdr_textures())
        {
            *hdr_references.entry(hdr.id()).or_default() += 1;
        }
        for handle in self
            .pictures
            .values()
            .map(|p| &p.handle)
            .chain(self.presentation.images.iter().map(|p| &p.handle))
            .chain(self.thumbs.values())
            .chain(self.details.textures())
            .chain(self.library.thumbnail_textures())
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
                if let Some(hdr) = &picture.hdr {
                    let count = hdr_references.get_mut(&hdr.id()).unwrap();
                    *count -= 1;
                    if *count == 0 {
                        bytes = bytes.saturating_sub(hdr.bytes());
                    }
                }
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
        if self.source_list_pending() {
            self.cancel_prefetch();
            return;
        }
        #[cfg(any(target_os = "ios", test))]
        if self.ios_photos_refreshing() {
            self.cancel_prefetch();
            return;
        }
        let settings = (
            self.settings.cache_image_limit,
            self.settings.cache_memory_mb,
            self.fit_limit,
        );
        if self.prefetch_settings != settings {
            self.prefetch_settings = settings;
            self.prefetch_blocked.clear();
            self.disk_warmed.clear();
            self.cancel_prefetch();
        }
        if !self.memory_policy.background_allowed()
            || self.renaming()
            || self.files.is_empty()
            || self.listing.is_some()
        {
            self.cancel_prefetch();
            return;
        }
        let interacting = self.touch_gesture.active() || self.mobile_swipe.is_some();
        // An interaction or a current page in flight only stops new work.
        // The decoder pauses wider speculative work by itself, and the next
        // page keeps decoding through a swipe.
        if interacting && self.prefetch_urgency > media::Urgency::Lead {
            self.cancel_prefetch();
        }
        if interacting
            || self.details.pending()
            || self.current_images_pending()
            || self.library.thumbnails_pending()
        {
            return;
        }
        // Avoid rebuilding the (potentially 1,000-page) working set on every
        // frame while foreground work or an interaction prevents prefetching.
        let targets = self.prefetch_targets();
        let wanted: HashSet<_> = targets
            .iter()
            .map(|t| self.files[t.index].clone())
            .collect();
        if self
            .prefetch_ticket
            .as_ref()
            .is_some_and(|(_, path, _)| !wanted.contains(path))
        {
            self.cancel_prefetch();
        }
        self.disk_warmed.retain(|path| wanted.contains(path));
        // Keep the working set centered on the requested pages in the current
        // randomized order. Mobile stops at the edges; desktop may wrap.
        let nearby = self.nearby_cache_paths();
        let desired: HashSet<_> = nearby.iter().collect();
        self.pictures.retain(|path, _| desired.contains(path));
        self.trim_cache(0);
        if self.prefetch_ticket.is_some() {
            return;
        }
        let reserved = media::prefetch_output_budget(self.fit_limit);
        let memory_full = self.pictures.len() >= self.settings.cache_image_limit
            || self.cache_bytes().saturating_add(reserved) > self.cache_budget();
        let range_thumbnails = self.range_thumbnails_pending();
        let next = targets.into_iter().find(|target| {
            let path = &self.files[target.index];
            !self.pictures.contains_key(path)
                && !self.failures.contains_key(path)
                && !self.prefetch_blocked.contains(path)
                && !(target.disk_only && self.disk_warmed.contains(path))
                && (target.disk_only || !memory_full)
        });
        let Some(target) = next else {
            return;
        };
        // The next page runs beside range thumbnails; anything further
        // waits until the thumbnails around the current page are ready.
        if target.urgency > media::Urgency::RangeThumbnail && range_thumbnails {
            return;
        }
        let path = self.files[target.index].clone();
        let id = if target.disk_only {
            self.media
                .request_disk_warm(path.clone(), self.fit_limit, target.urgency)
        } else {
            self.media
                .request_prefetch_with(path.clone(), self.fit_limit, target.urgency)
        };
        self.prefetch_urgency = target.urgency;
        self.prefetch_ticket = Some((id, path, self.fit_limit));
    }

    pub(super) fn refresh_folder(&mut self, mut files: Vec<PathBuf>, changed: &[PathBuf]) {
        if self.random {
            files = reconcile_random(&self.files, &files);
        } else {
            files = self.sorted_files(files);
        }
        if self.files != files || !changed.is_empty() {
            self.check_navigation = None;
            self.mobile_swipe = None;
            self.touch_gesture.cancel();
        }
        let focused_path = self.files.get(self.focus_index).cloned();
        let focus_index = refreshed_focus_index(&self.files, self.focus_index, &files);
        let live: HashSet<_> = files.iter().collect();
        self.checked.retain(|path| live.contains(path));
        for path in changed {
            self.details.invalidate(path);
        }
        let changed: HashSet<_> = changed.iter().collect();
        let valid = |p: &PathBuf| live.contains(p) && !changed.contains(p);
        self.pictures.retain(|p, _| valid(p));
        self.page_sizes.retain(|p, _| valid(p));
        self.prefetch_blocked.clear();
        self.disk_warmed.retain(valid);
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
        if self.files != files {
            self.spread_shifted = false;
        }
        self.span_cache.get_mut().take();
        self.files = files;
        if self.files.is_empty() {
            self.last_error.clear();
            self.presentation = Presentation::default();
            self.transition_waiting = false;
        }
    }

    /// A prefetched result is still useful at this position and size.
    fn prefetched_wanted(&self, path: &PathBuf, requested: u32) -> bool {
        self.memory_policy.background_allowed()
            && !self.pictures.contains_key(path)
            && requested == self.fit_limit
            && self.nearby_cache_paths().contains(path)
    }

    /// Prefetched textures never upload during a swipe or gesture, and at most
    /// one large (over 4 MP) uploads per frame. The current page is exempt: it
    /// arrives as `MediaEvent::Image`. Held frames wait in a short queue.
    pub(super) fn offer_prefetched(
        &mut self,
        ctx: &egui::Context,
        id: u64,
        path: PathBuf,
        frame: media::Frame,
        requested: u32,
    ) {
        let large = frame.image.pixels.len() > LARGE_UPLOAD_PIXELS;
        let interacting = self.touch_gesture.active() || self.mobile_swipe.is_some();
        if interacting || (large && self.large_uploads > 0) {
            if self.held_prefetch.len() >= HELD_PREFETCH {
                // The oldest is furthest behind; its disk cache entry remains.
                self.held_prefetch.pop_front();
            }
            self.held_prefetch.push_back((id, path, frame, requested));
            ctx.request_repaint();
            return;
        }
        let bytes =
            frame.image.pixels.len() * 4 + frame.hdr.as_ref().map_or(0, |hdr| hdr.bytes() * 3 / 2);
        if self.pictures.len() >= self.settings.cache_image_limit
            || self.cache_bytes().saturating_add(bytes) > self.cache_budget()
        {
            self.prefetch_blocked.insert(path);
            return;
        }
        self.large_uploads += usize::from(large);
        self.span_cache.get_mut().take();
        self.page_sizes.insert(path.clone(), frame.original_size);
        self.pictures.insert(
            path,
            Picture {
                hdr: frame
                    .hdr
                    .and_then(|hdr| {
                        self.render_state
                            .as_ref()
                            .and_then(|state| crate::ui::hdr::Texture::new(state, hdr))
                    })
                    .map(std::sync::Arc::new),
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

    /// Uploads held prefetch results once the policy allows, oldest first.
    pub(super) fn release_held_prefetch(&mut self, ctx: &egui::Context) {
        while !self.touch_gesture.active() && self.mobile_swipe.is_none() {
            let Some((id, path, frame, requested)) = self.held_prefetch.pop_front() else {
                return;
            };
            if !self.prefetched_wanted(&path, requested) {
                continue;
            }
            let large = frame.image.pixels.len() > LARGE_UPLOAD_PIXELS;
            if large && self.large_uploads > 0 {
                self.held_prefetch.push_front((id, path, frame, requested));
                ctx.request_repaint();
                return;
            }
            self.offer_prefetched(ctx, id, path, frame, requested);
        }
    }

    pub(super) fn collect(&mut self, ctx: &egui::Context) {
        self.large_uploads = 0;
        self.release_held_prefetch(ctx);
        while let Some(event) = self.media.try_recv() {
            #[cfg(any(target_os = "ios", target_os = "android", test))]
            if let MediaEvent::Image { path, .. }
            | MediaEvent::Thumbnail { path, .. }
            | MediaEvent::Prefetched { path, .. } = &event
            {
                // Reading an original can reveal a size PhotoKit did not list.
                // This lookup is memory-only, including when decoding failed.
                if let Some(bytes) = crate::core::documents::deferred_size(path)
                    && let Some(stamp) = self.sorting.stamps.get_mut(path)
                    && stamp.bytes != bytes
                {
                    stamp.bytes = bytes;
                    self.sorting.deferred_size_pending = true;
                }
            }
            match event {
                MediaEvent::Folder { id, path, result }
                    if self.listing == Some(id) && self.folder.as_ref() == Some(&path) =>
                {
                    self.listing = None;
                    match result {
                        Ok(files) => {
                            self.sorting.stamps = self.media.folder_stamps(id);
                            #[cfg(any(target_os = "ios", target_os = "android", test))]
                            if let Some(selection) = &self.mobile_selection {
                                selection.apply_sort_metadata(&mut self.sorting.stamps);
                            }
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
                        Ok(update) => {
                            self.sorting.stamps = self.media.folder_stamps(id);
                            #[cfg(any(target_os = "ios", target_os = "android", test))]
                            if let Some(selection) = &self.mobile_selection {
                                selection.apply_sort_metadata(&mut self.sorting.stamps);
                            }
                            self.refresh_folder(update.files, &update.changed);
                        }
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
                                    hdr: frame
                                        .hdr
                                        .and_then(|hdr| {
                                            self.render_state.as_ref().and_then(|state| {
                                                crate::ui::hdr::Texture::new(state, hdr)
                                            })
                                        })
                                        .map(std::sync::Arc::new),
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
                            let bytes = frame.image.pixels.len() * 4
                                + frame.hdr.as_ref().map_or(0, |hdr| hdr.bytes() * 3 / 2);
                            self.trim_cache(bytes);
                            if self.cache_bytes().saturating_add(bytes) > self.cache_budget() {
                                continue;
                            }
                            // Keep every visible thumbnail even on very wide displays.
                            // The shared byte budget still bounds memory use.
                            let visible = thumbnail_range(
                                self.files.len(),
                                self.focus_index,
                                ctx.viewport_rect().width() - THUMBNAIL_MARGIN * 2.,
                                ctx.global_style().spacing.item_spacing.x,
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
                    if !self.prefetched_wanted(&path, requested) {
                        continue;
                    }
                    match result {
                        Ok(frame) => self.offer_prefetched(ctx, id, path, frame, requested),
                        Err(_) => {
                            // Retry through the visible path only; a corrupt
                            // neighbor must not create an endless background loop.
                            self.prefetch_blocked.insert(path);
                        }
                    }
                }
                MediaEvent::Warmed { id, path, result }
                    if self
                        .prefetch_ticket
                        .as_ref()
                        .is_some_and(|t| t.0 == id && t.1 == path) =>
                {
                    let (_, _, requested) = self.prefetch_ticket.take().unwrap();
                    if requested != self.fit_limit {
                        continue;
                    }
                    if result.is_ok() {
                        self.disk_warmed.insert(path);
                    } else {
                        self.prefetch_blocked.insert(path);
                    }
                }
                MediaEvent::Warmed { id, .. } if self.prefill_finished(id) => {}
                _ => {}
            }
        }
        #[cfg(any(target_os = "ios", target_os = "android", test))]
        if self.sorting.deferred_size_pending
            && !self.touch_gesture.active()
            && self.mobile_swipe.is_none()
        {
            self.sorting.deferred_size_pending = false;
            if self.settings.sort_by == "size" {
                self.resort_preserving_focus();
            }
        }
        self.queue_images();
        self.trim_cache(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_submits_current_page_first_keeps_near_work_and_resumes_when_ready() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            None,
            Err(String::new()),
        );
        app.apply_memory_sample(Some(crate::platform::system_memory::Snapshot {
            total: 4 * 1024 * 1024 * 1024,
            available: 2 * 1024 * 1024 * 1024,
        }));
        app.files = vec!["first.png".into(), "next.png".into(), "neighbor.png".into()];
        let old_thumb = app.media.request_thumbnail(app.files[0].clone(), 192);
        app.thumb_ticket = Some((old_thumb, app.files[0].clone()));
        let old_prefetch = app.media.request_prefetch(app.files[1].clone(), 2048);
        app.prefetch_ticket = Some((old_prefetch, app.files[1].clone(), 2048));

        app.focus(1);
        assert_eq!(app.tickets[0].as_ref().map(|t| &t.1), Some(&app.files[1]));
        // A one-page step keeps the strip thumbnail; the old prefetch of the
        // new current page yields to the visible decoder.
        assert_eq!(app.thumb_ticket, Some((old_thumb, app.files[0].clone())));
        assert!(app.prefetch_ticket.is_none());
        assert!(app.current_images_pending());
        let draw = |app: &mut Viewer| {
            let _ = ctx.run_pass(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(390., 844.),
                    )),
                    ..Default::default()
                },
                |ctx| app.thumbnail_bar(ctx),
            );
            app.queue_prefetch();
        };
        // Do not collect the decoder: the current page deliberately remains
        // pending, so nothing new starts and the kept request is not restarted.
        for _ in 0..4 {
            draw(&mut app);
            assert_eq!(app.thumb_ticket.as_ref().map(|t| t.0), Some(old_thumb));
            assert!(app.prefetch_ticket.is_none());
        }
        app.media.clear_slot(0);
        app.tickets[0] = None;
        app.pictures.insert(
            app.files[1].clone(),
            Picture {
                hdr: None,
                animation: None,
                handle: ctx.load_texture(
                    "ready-page",
                    egui::ColorImage::filled([8, 4], Color32::WHITE),
                    egui::TextureOptions::LINEAR,
                ),
                original: [8, 4],
                requested: 2048,
                last_used: 0,
            },
        );
        assert!(!app.current_images_pending());
        app.touch_gesture.start(egui::pos2(200., 100.));
        draw(&mut app);
        assert_eq!(app.thumb_ticket.as_ref().map(|t| t.0), Some(old_thumb));
        assert!(app.prefetch_ticket.is_none(), "a gesture starts nothing");
        app.touch_gesture.cancel();
        draw(&mut app);
        assert!(
            app.thumb_ticket.is_some(),
            "ready pages must not starve thumbnails"
        );
        // The next page is requested beside the strip; the shared decoder
        // still runs the on-screen thumbnail first.
        assert_eq!(
            app.prefetch_ticket.as_ref().map(|t| &t.1),
            Some(&app.files[2])
        );
        assert_eq!(app.prefetch_urgency, media::Urgency::Lead);
        // A known failed page has no foreground work. Superseded decoding must
        // not keep its old ticket and hold the background gate closed.
        app.focus(0);
        assert!(app.tickets[0].is_some());
        app.failures
            .insert(app.files[2].clone(), "Unreadable image".into());
        app.focus(2);
        assert!(app.tickets.iter().all(Option::is_none));
        assert!(!app.current_images_pending());
    }
}
