#![no_std]
#![no_main]

extern crate alloc;

use alloc::{format, string::String, vec, vec::Vec};
use sun_font::{draw_text, draw_text_vcenter, FontRole, TextStyle, VecFont};
use sunlight_deviced::{
    failure_stage_label, list_timeout, load_record_timeout, state_display_label, DeviceId,
    InventoryClientError, InventoryRecord, InventorySummary, ShortName,
};
use sunlight_devices::{
    device_display_name, DeviceClassId, InventorySnapshot, PresentationState, SnapshotBuildError,
    TreeNodeId, TreeSnapshot,
};
use sunlight_ipc::{
    debug_log, nameserver_lookup_timeout, process_yield, CapabilityToken, HardwareBus,
    HardwareState, ProcessExit,
};
use sunlight_ui::image::TgaImage;
use sunlight_ui::widgets::{
    BadgeKind, Panel, StatusBadge, StatusBar, TreeItem, TreeModel, TreeView, TreeViewState,
};
use sunlight_ui::{
    request_close, App, Canvas, Event, Point, Rect, Theme, Window, WindowConfig, WindowDecoration,
};

static FONT_SMALL: VecFont = VecFont(FontRole::UiSmall);

static ICON_APP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_app.tga"));
static ICON_REFRESH: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_refresh.tga"));
static ICON_DISPLAY: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_display.tga"));
static ICON_NETWORK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_network.tga"));
static ICON_STORAGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_storage.tga"));
static ICON_AUDIO: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_audio.tga"));
static ICON_KEYBOARD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_keyboard.tga"));
static ICON_MOUSE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_mouse.tga"));
static ICON_USB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_usb.tga"));
static ICON_SYSTEM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_system.tga"));
static ICON_BRIDGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_bridge.tga"));
static ICON_OTHER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_other.tga"));
static ICON_WARNING: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_warning.tga"));
static ICON_SECTION: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_section.tga"));

const WIN_W: u32 = 1040;
const WIN_H: u32 = 680;
const TOOLBAR_H: u32 = 48;
const STATUS_H: u32 = StatusBar::HEIGHT;
const TREE_W: u32 = 350;
const GAP: i32 = 10;
const PAD: i32 = 10;
const KEY_ESC: u8 = 0x01;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_LEFT: u8 = 0x4b;
const KEY_RIGHT: u8 = 0x4d;
const KEY_HOME: u8 = 0x47;
const KEY_END: u8 = 0x4f;
const KEY_PGUP: u8 = 0x49;
const KEY_PGDN: u8 = 0x51;
const IPC_LOOKUP_TIMEOUT_MS: u64 = 30;
const UI_INVENTORY_TIMEOUT_MS: u64 = 16;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[DEVICES] panic\n");
    loop {
        process_yield();
    }
}

#[derive(Clone, Copy)]
struct Icons {
    app: TgaImage,
    refresh: TgaImage,
    display: TgaImage,
    network: TgaImage,
    storage: TgaImage,
    audio: TgaImage,
    keyboard: TgaImage,
    mouse: TgaImage,
    usb: TgaImage,
    system: TgaImage,
    bridge: TgaImage,
    other: TgaImage,
    warning: TgaImage,
    section: TgaImage,
}

impl Icons {
    fn load() -> Self {
        Self {
            app: TgaImage::parse(ICON_APP).unwrap(),
            refresh: TgaImage::parse(ICON_REFRESH).unwrap(),
            display: TgaImage::parse(ICON_DISPLAY).unwrap(),
            network: TgaImage::parse(ICON_NETWORK).unwrap(),
            storage: TgaImage::parse(ICON_STORAGE).unwrap(),
            audio: TgaImage::parse(ICON_AUDIO).unwrap(),
            keyboard: TgaImage::parse(ICON_KEYBOARD).unwrap(),
            mouse: TgaImage::parse(ICON_MOUSE).unwrap(),
            usb: TgaImage::parse(ICON_USB).unwrap(),
            system: TgaImage::parse(ICON_SYSTEM).unwrap(),
            bridge: TgaImage::parse(ICON_BRIDGE).unwrap(),
            other: TgaImage::parse(ICON_OTHER).unwrap(),
            warning: TgaImage::parse(ICON_WARNING).unwrap(),
            section: TgaImage::parse(ICON_SECTION).unwrap(),
        }
    }

    fn class(self, class: DeviceClassId) -> TgaImage {
        match class {
            DeviceClassId::Display => self.display,
            DeviceClassId::Network => self.network,
            DeviceClassId::Storage => self.storage,
            DeviceClassId::Audio => self.audio,
            DeviceClassId::Input => self.keyboard,
            DeviceClassId::Usb => self.usb,
            DeviceClassId::Bridge => self.bridge,
            DeviceClassId::System | DeviceClassId::Communication => self.system,
            DeviceClassId::Other(_, _) => self.other,
        }
    }

    fn device(self, record: InventoryRecord) -> TgaImage {
        if matches!(record.summary.bus(), HardwareBus::Ps2) && record.summary.subclass() == 0x02 {
            self.mouse
        } else {
            self.class(DeviceClassId::from_record(record))
        }
    }
}

#[derive(Debug)]
enum RefreshPhase {
    Idle,
    Lookup,
    List {
        capability: CapabilityToken,
        next_index: usize,
        expected_total: Option<usize>,
    },
    Fields {
        capability: CapabilityToken,
        summaries: Vec<InventorySummary>,
        records: Vec<InventoryRecord>,
        next_index: usize,
    },
}

struct DeviceTreeModel<'a> {
    snapshot: &'a InventorySnapshot,
    tree: &'a TreeSnapshot,
    icons: Icons,
}

impl<'a> DeviceTreeModel<'a> {
    fn new(snapshot: &'a InventorySnapshot, tree: &'a TreeSnapshot, icons: Icons) -> Self {
        Self {
            snapshot,
            tree,
            icons,
        }
    }

    fn group_index(&self, class_key: u64) -> Option<usize> {
        self.snapshot
            .groups
            .iter()
            .position(|group| group.class.stable_key() == class_key)
    }
}

impl TreeModel for DeviceTreeModel<'_> {
    type Id = TreeNodeId;

    fn roots(&self) -> &[Self::Id] {
        self.tree.roots()
    }

    fn parent(&self, id: Self::Id) -> Option<Self::Id> {
        let TreeNodeId::Device(key) = id else {
            return None;
        };
        let record = self.snapshot.record(key)?;
        Some(TreeNodeId::Class(
            DeviceClassId::from_record(*record).stable_key(),
        ))
    }

    fn children(&self, id: Self::Id) -> &[Self::Id] {
        let TreeNodeId::Class(class_key) = id else {
            return &[];
        };
        self.group_index(class_key)
            .map(|index| self.tree.children(index))
            .unwrap_or(&[])
    }

    fn item(&self, id: Self::Id) -> TreeItem {
        match id {
            TreeNodeId::Class(class_key) => {
                let group = &self.snapshot.groups[self.group_index(class_key).unwrap()];
                TreeItem::new(group.class.label())
                    .with_image_icon(self.icons.class(group.class))
                    .with_secondary_text(format!("{}", group.devices.len()))
            }
            TreeNodeId::Device(key) => {
                let record = *self.snapshot.record(key).unwrap();
                let mut item = TreeItem::new(device_display_name(record))
                    .with_image_icon(self.icons.device(record));
                if matches!(
                    record.state(),
                    HardwareState::ProbeFailed | HardwareState::NoDriver | HardwareState::Unknown
                ) {
                    item = item
                        .with_secondary_text(state_display_label(record.state()))
                        .with_status_image_icon(self.icons.warning);
                }
                if matches!(record.state(), HardwareState::Disabled) {
                    item = item.disabled();
                }
                item
            }
        }
    }
}

#[derive(Clone, Copy)]
enum RefreshFailure {
    ServiceUnavailable,
    MalformedReply,
    InventoryChanged,
    Allocation,
}

const fn refresh_failure_from_snapshot_error(error: SnapshotBuildError) -> RefreshFailure {
    match error {
        SnapshotBuildError::TooManyRecords | SnapshotBuildError::DuplicateDevice => {
            RefreshFailure::MalformedReply
        }
        SnapshotBuildError::Allocation => RefreshFailure::Allocation,
    }
}

#[derive(Default)]
struct RefreshDiagnostics {
    attempts: u64,
    successes: u64,
    service_failures: u64,
    malformed_replies: u64,
    allocation_failures: u64,
    stale_candidates: u64,
    discarded_candidates: u64,
    current_device_count: usize,
    last_error: Option<RefreshFailure>,
}

struct DevicesApp {
    icons: Icons,
    presentation: PresentationState,
    tree: Option<TreeSnapshot>,
    tree_state: TreeViewState<TreeNodeId>,
    hovered: Option<TreeNodeId>,
    phase: RefreshPhase,
    refresh_queued: bool,
    refresh_hovered: bool,
    focused: bool,
    detail_scroll: usize,
    status_text: String,
    diagnostics: RefreshDiagnostics,
}

impl DevicesApp {
    fn new() -> Self {
        Self {
            icons: Icons::load(),
            presentation: PresentationState::default(),
            tree: None,
            tree_state: TreeViewState::new(),
            hovered: None,
            phase: RefreshPhase::Idle,
            refresh_queued: false,
            refresh_hovered: false,
            focused: true,
            detail_scroll: 0,
            status_text: String::from("Loading hardware inventory…"),
            diagnostics: RefreshDiagnostics::default(),
        }
    }

    fn layout() -> (Rect, Rect, Rect, Rect, Rect) {
        let toolbar = Rect::new(0, 0, WIN_W, TOOLBAR_H);
        let status = Rect::new(0, WIN_H as i32 - STATUS_H as i32, WIN_W, STATUS_H);
        let content = Rect::new(
            PAD,
            TOOLBAR_H as i32 + PAD,
            WIN_W - (PAD as u32 * 2),
            WIN_H - TOOLBAR_H - STATUS_H - (PAD as u32 * 2),
        );
        let tree = Rect::new(content.x, content.y, TREE_W, content.h);
        let details = Rect::new(
            tree.right() + GAP,
            content.y,
            content.w.saturating_sub(TREE_W + GAP as u32),
            content.h,
        );
        let refresh = Rect::new(WIN_W as i32 - 122, 9, 108, 30);
        (toolbar, status, tree, details, refresh)
    }

    fn is_refreshing(&self) -> bool {
        !matches!(self.phase, RefreshPhase::Idle)
    }

    fn request_refresh(&mut self) {
        if self.is_refreshing() {
            self.refresh_queued = true;
            return;
        }
        self.diagnostics.attempts = self.diagnostics.attempts.saturating_add(1);
        self.phase = RefreshPhase::Lookup;
        self.status_text = if self.presentation.snapshot.is_some() {
            String::from("Refreshing hardware inventory…")
        } else {
            String::from("Loading hardware inventory…")
        };
    }

    fn fail_refresh(&mut self, failure: RefreshFailure) {
        self.diagnostics.discarded_candidates =
            self.diagnostics.discarded_candidates.saturating_add(1);
        self.diagnostics.last_error = Some(failure);
        match failure {
            RefreshFailure::ServiceUnavailable => {
                self.diagnostics.service_failures =
                    self.diagnostics.service_failures.saturating_add(1);
            }
            RefreshFailure::MalformedReply => {
                self.diagnostics.malformed_replies =
                    self.diagnostics.malformed_replies.saturating_add(1);
            }
            RefreshFailure::InventoryChanged => {
                self.diagnostics.stale_candidates =
                    self.diagnostics.stale_candidates.saturating_add(1);
            }
            RefreshFailure::Allocation => {
                self.diagnostics.allocation_failures =
                    self.diagnostics.allocation_failures.saturating_add(1);
            }
        }
        let message = if self.presentation.snapshot.is_some() {
            "Refresh failed — showing previous device data"
        } else {
            match failure {
                RefreshFailure::ServiceUnavailable => "deviced unavailable",
                RefreshFailure::MalformedReply => "Malformed inventory reply",
                RefreshFailure::InventoryChanged => "Inventory changed during refresh",
                RefreshFailure::Allocation => "Not enough memory to refresh inventory",
            }
        };
        self.presentation.fail_refresh(message);
        self.phase = RefreshPhase::Idle;
        self.status_text = String::from(message);
        self.start_queued_refresh();
    }

    fn finish_refresh(&mut self, records: Vec<InventoryRecord>) {
        let snapshot = match InventorySnapshot::try_new(records) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.fail_refresh(refresh_failure_from_snapshot_error(error));
                return;
            }
        };
        let tree = match TreeSnapshot::try_new(&snapshot) {
            Ok(tree) => tree,
            Err(error) => {
                self.fail_refresh(refresh_failure_from_snapshot_error(error));
                return;
            }
        };
        if let Err(error) = self.presentation.try_apply_snapshot(snapshot) {
            self.fail_refresh(refresh_failure_from_snapshot_error(error));
            return;
        }
        self.tree = Some(tree);
        self.sync_tree_state();
        self.status_text = self.summary_text();
        self.phase = RefreshPhase::Idle;
        self.diagnostics.successes = self.diagnostics.successes.saturating_add(1);
        self.diagnostics.current_device_count = self
            .presentation
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.records.len());
        self.diagnostics.last_error = None;
        self.start_queued_refresh();
    }

    fn start_queued_refresh(&mut self) {
        if self.refresh_queued {
            self.refresh_queued = false;
            self.request_refresh();
        }
    }

    fn sync_tree_state(&mut self) {
        let Some(snapshot) = self.presentation.snapshot.as_ref() else {
            return;
        };
        let Some(tree) = self.tree.as_ref() else {
            return;
        };
        let model = DeviceTreeModel::new(snapshot, tree, self.icons);
        for class_key in self.presentation.expanded_classes.iter().copied() {
            self.tree_state.expand(TreeNodeId::Class(class_key));
        }
        self.tree_state
            .set_selected(self.presentation.selected_device.map(TreeNodeId::Device));
        let _ = self.tree_state.rebuild_rows(&model);
        self.presentation.selected_device = match self.tree_state.selected_id() {
            Some(TreeNodeId::Device(key)) => Some(key),
            _ => None,
        };
    }

    fn sync_presentation_from_tree(&mut self) {
        self.presentation.selected_device = match self.tree_state.selected_id() {
            Some(TreeNodeId::Device(key)) => Some(key),
            _ => None,
        };
        self.presentation.expanded_classes.clear();
        if let Some(snapshot) = self.presentation.snapshot.as_ref() {
            for group in &snapshot.groups {
                let key = group.class.stable_key();
                if self.tree_state.is_expanded(TreeNodeId::Class(key)) {
                    self.presentation.expanded_classes.push(key);
                }
            }
        }
    }

    fn advance_refresh(&mut self) -> bool {
        let phase = core::mem::replace(&mut self.phase, RefreshPhase::Idle);
        match phase {
            RefreshPhase::Idle => false,
            RefreshPhase::Lookup => {
                let Some(capability) = nameserver_lookup_timeout("deviced", IPC_LOOKUP_TIMEOUT_MS)
                else {
                    self.fail_refresh(RefreshFailure::ServiceUnavailable);
                    return true;
                };
                self.phase = RefreshPhase::List {
                    capability,
                    next_index: 0,
                    expected_total: None,
                };
                true
            }
            RefreshPhase::List {
                capability,
                next_index,
                expected_total,
            } => match list_timeout(capability, next_index, UI_INVENTORY_TIMEOUT_MS) {
                Ok(summary) => {
                    let total = expected_total.unwrap_or(summary.total);
                    if summary.total != total || total == 0 || next_index >= total {
                        self.fail_refresh(RefreshFailure::MalformedReply);
                    } else {
                        let mut summaries = Vec::new();
                        let mut records = Vec::new();
                        if summaries.try_reserve_exact(total).is_err()
                            || records.try_reserve_exact(total).is_err()
                        {
                            self.fail_refresh(RefreshFailure::Allocation);
                            return true;
                        }
                        summaries.push(summary);
                        self.phase = RefreshPhase::Fields {
                            capability,
                            summaries,
                            records,
                            next_index: 0,
                        };
                    }
                    true
                }
                Err(InventoryClientError::NotFound) if next_index == 0 => {
                    self.finish_refresh(Vec::new());
                    true
                }
                Err(InventoryClientError::MalformedReply) => {
                    self.fail_refresh(RefreshFailure::MalformedReply);
                    true
                }
                Err(InventoryClientError::NotFound) => {
                    self.fail_refresh(RefreshFailure::InventoryChanged);
                    true
                }
                Err(InventoryClientError::Transport(_)) => {
                    self.fail_refresh(RefreshFailure::ServiceUnavailable);
                    true
                }
            },
            RefreshPhase::Fields {
                capability,
                mut summaries,
                mut records,
                next_index,
            } => {
                let expected_total = summaries.first().map_or(0, |summary| summary.total);
                if summaries.len() < expected_total {
                    match list_timeout(capability, summaries.len(), UI_INVENTORY_TIMEOUT_MS) {
                        Ok(summary) if summary.total == expected_total => {
                            summaries.push(summary);
                            self.phase = RefreshPhase::Fields {
                                capability,
                                summaries,
                                records,
                                next_index,
                            };
                        }
                        Ok(_) | Err(InventoryClientError::NotFound) => {
                            self.fail_refresh(RefreshFailure::InventoryChanged)
                        }
                        Err(InventoryClientError::MalformedReply) => {
                            self.fail_refresh(RefreshFailure::MalformedReply)
                        }
                        Err(InventoryClientError::Transport(_)) => {
                            self.fail_refresh(RefreshFailure::ServiceUnavailable)
                        }
                    }
                    return true;
                }
                let Some(summary) = summaries.get(next_index).copied() else {
                    self.finish_refresh(records);
                    return true;
                };
                match load_record_timeout(capability, summary, UI_INVENTORY_TIMEOUT_MS) {
                    Ok(record) => {
                        records.push(record);
                        let next_index = next_index + 1;
                        if next_index >= summaries.len() {
                            self.finish_refresh(records);
                        } else {
                            self.phase = RefreshPhase::Fields {
                                capability,
                                summaries,
                                records,
                                next_index,
                            };
                        }
                    }
                    Err(InventoryClientError::NotFound) => {
                        self.fail_refresh(RefreshFailure::InventoryChanged)
                    }
                    Err(InventoryClientError::MalformedReply) => {
                        self.fail_refresh(RefreshFailure::MalformedReply)
                    }
                    Err(InventoryClientError::Transport(_)) => {
                        self.fail_refresh(RefreshFailure::ServiceUnavailable)
                    }
                }
                true
            }
        }
    }

    fn summary_text(&self) -> String {
        let Some(snapshot) = self.presentation.snapshot.as_ref() else {
            return String::from("No inventory loaded");
        };
        if snapshot.records.is_empty() {
            return String::from("No hardware devices reported");
        }
        let counts = snapshot.status_counts();
        let mut parts = vec![format!("{} devices", counts.total)];
        for (count, label) in [
            (counts.active, "active"),
            (counts.loaded, "loaded"),
            (counts.probe_failed, "probe failed"),
            (counts.no_driver, "without driver"),
            (counts.disabled, "disabled"),
            (counts.unknown, "unknown"),
        ] {
            if count != 0 {
                parts.push(format!("{count} {label}"));
            }
        }
        parts.join(" • ")
    }

    fn draw_toolbar(&self, canvas: &mut Canvas, theme: &Theme, toolbar: Rect, refresh: Rect) {
        canvas.fill_rect(toolbar, theme.panel);
        canvas.hbar(0, toolbar.bottom() - 1, toolbar.w, 1, theme.border);
        canvas.draw_tga_icon_tinted(&self.icons.app, Rect::new(14, 8, 32, 32), theme.accent);
        draw_text(
            canvas,
            "Sunlight Devices",
            54,
            14,
            &TextStyle::new(FontRole::UiTitle, theme.text),
        );
        let button_color = if self.is_refreshing() {
            theme.panel_alt
        } else {
            theme.accent.darken(145)
        };
        canvas.fill_rect(refresh, button_color);
        canvas.draw_rect(refresh, theme.border);
        canvas.draw_tga_icon_tinted(
            &self.icons.refresh,
            Rect::new(refresh.x + 8, refresh.y + 5, 20, 20),
            if self.is_refreshing() {
                theme.text_dim
            } else {
                theme.accent
            },
        );
        draw_text_vcenter(
            canvas,
            if self.is_refreshing() {
                "Refreshing"
            } else {
                "Refresh"
            },
            refresh.x + 34,
            refresh.y,
            refresh.h,
            &TextStyle::new(
                FontRole::UiRegular,
                if self.is_refreshing() {
                    theme.text_dim
                } else {
                    theme.text
                },
            ),
        );
        if self.refresh_hovered {
            let tooltip = Rect::new(refresh.x - 48, refresh.bottom() + 4, 156, 24);
            canvas.fill_rect(tooltip, theme.panel_alt);
            canvas.draw_rect(tooltip, theme.border);
            draw_text_vcenter(
                canvas,
                "Refresh device inventory",
                tooltip.x + 8,
                tooltip.y,
                tooltip.h,
                &TextStyle::new(FontRole::UiSmall, theme.text),
            );
        }
    }

    fn draw_tree(&mut self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        let panel = Panel::with_title(rect, "Device tree");
        panel.draw(canvas, theme);
        let inner = panel.content_rect().inset(4);
        let Some(snapshot) = self.presentation.snapshot.as_ref() else {
            draw_text(
                canvas,
                "Loading from deviced…",
                inner.x + 8,
                inner.y + 10,
                &TextStyle::new(FontRole::UiRegular, theme.text_dim),
            );
            return;
        };
        if snapshot.records.is_empty() {
            draw_text(
                canvas,
                "No devices reported.",
                inner.x + 8,
                inner.y + 10,
                &TextStyle::new(FontRole::UiRegular, theme.text_dim),
            );
            return;
        }
        let Some(tree_snapshot) = self.tree.as_ref() else {
            return;
        };
        let model = DeviceTreeModel::new(snapshot, tree_snapshot, self.icons);
        let rows = self.tree_state.rebuild_rows(&model);
        let tree = TreeView::new(inner, &rows)
            .with_font(&FONT_SMALL)
            .with_scroll_offset(self.tree_state.scroll_offset())
            .with_focus(self.focused)
            .with_hovered(self.hovered);
        self.tree_state
            .clamp_scroll(rows.len(), tree.visible_row_count());
        tree.draw(canvas, theme);
    }

    fn draw_details(&self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        let panel = Panel::with_title(rect, "Device details");
        panel.draw(canvas, theme);
        let inner = panel.content_rect().inset(14);
        let Some(record) = self.presentation.selected_record().copied() else {
            canvas.draw_tga_icon_tinted(
                &self.icons.app,
                Rect::new(inner.x + 8, inner.y + 22, 54, 54),
                theme.text_dim,
            );
            draw_text(
                canvas,
                "Select a device to view its hardware and driver details.",
                inner.x + 76,
                inner.y + 30,
                &TextStyle::new(FontRole::UiRegular, theme.text_dim),
            );
            if let Some(snapshot) = self.presentation.snapshot.as_ref() {
                draw_text(
                    canvas,
                    &format!(
                        "{} devices in {} classes",
                        snapshot.records.len(),
                        snapshot.groups.len()
                    ),
                    inner.x + 76,
                    inner.y + 50,
                    &TextStyle::new(FontRole::UiSmall, theme.text_dim),
                );
            }
            return;
        };

        let header_icon = self.icons.device(record);
        canvas.draw_tga_icon_tinted(
            &header_icon,
            Rect::new(inner.x, inner.y, 46, 46),
            theme.accent,
        );
        let name = device_display_name(record);
        draw_text(
            canvas,
            &name,
            inner.x + 58,
            inner.y + 2,
            &TextStyle::new(FontRole::UiTitle, theme.text),
        );
        let kind = match record.state() {
            HardwareState::Active => BadgeKind::Ok,
            HardwareState::Loaded => BadgeKind::Accent,
            HardwareState::ProbeFailed => BadgeKind::Danger,
            HardwareState::NoDriver => BadgeKind::Warn,
            HardwareState::Disabled | HardwareState::Unknown => BadgeKind::Dim,
        };
        StatusBadge::new(inner.x + 58, inner.y + 27, kind)
            .with_label(state_display_label(record.state()))
            .draw(canvas, theme);
        draw_text(
            canvas,
            &format!("{}", DeviceId(record.key())),
            inner.right() - 160,
            inner.y + 27,
            &TextStyle::new(FontRole::MonoRegular, theme.text_dim),
        );

        let mut y = inner.y + 64 - (self.detail_scroll as i32 * 18);
        y = self.draw_section(canvas, theme, inner, y, "General", self.icons.section);
        y = self.draw_row(
            canvas,
            theme,
            inner,
            y,
            "Class",
            DeviceClassId::from_record(record).label(),
        );
        y = self.draw_row(
            canvas,
            theme,
            inner,
            y,
            "Bus",
            sunlight_deviced::bus_label(record.summary.bus()),
        );
        y = self.draw_row(
            canvas,
            theme,
            inner,
            y,
            "Address",
            &format!("{}", DeviceId(record.key())),
        );
        y = self.draw_row(
            canvas,
            theme,
            inner,
            y,
            "Discovery",
            match record.summary.bus() {
                HardwareBus::Pci => "PCI boot enumeration",
                HardwareBus::Ps2 => "PS/2 controller",
                HardwareBus::Platform => "platform registration",
                HardwareBus::Unknown => "unknown",
            },
        );
        if let (Some(vendor), Some(device)) =
            (record.summary.vendor_id(), record.summary.device_id())
        {
            y = self.draw_row(
                canvas,
                theme,
                inner,
                y,
                "Vendor/device",
                &format!("{vendor:04x}:{device:04x}"),
            );
        }
        if let (Some(vendor), Some(device)) =
            (record.subsystem_vendor_id(), record.subsystem_device_id())
        {
            y = self.draw_row(
                canvas,
                theme,
                inner,
                y,
                "Subsystem",
                &format!("{vendor:04x}:{device:04x}"),
            );
        }
        y = self.draw_row(
            canvas,
            theme,
            inner,
            y,
            "Revision",
            &format!("{:02x}", record.summary.revision()),
        );

        y = self.draw_section(canvas, theme, inner, y + 8, "Driver", self.icons.system);
        if record.matched_driver != 0 {
            y = self.draw_row(
                canvas,
                theme,
                inner,
                y,
                "Matched",
                &format!("{}", ShortName(record.matched_driver)),
            );
        }
        if record.bound_driver != 0 {
            y = self.draw_row(
                canvas,
                theme,
                inner,
                y,
                "Bound",
                &format!("{}", ShortName(record.bound_driver)),
            );
        }
        y = self.draw_row(
            canvas,
            theme,
            inner,
            y,
            "State",
            state_display_label(record.state()),
        );
        if record.failure_stage() != sunlight_ipc::HardwareFailureStage::None {
            y = self.draw_row(
                canvas,
                theme,
                inner,
                y,
                "Failure stage",
                failure_stage_label(record.failure_stage()),
            );
        }
        if record.error_code != 0 {
            if y < inner.bottom() {
                canvas.draw_tga_icon_tinted(
                    &self.icons.warning,
                    Rect::new(inner.x + 116, y - 2, 18, 18),
                    theme.danger,
                );
            }
            y = self.draw_row(
                canvas,
                theme,
                inner,
                y,
                "Error code",
                &format!("{}", record.error_code),
            );
            y = self.draw_row(
                canvas,
                theme,
                inner,
                y,
                "Diagnostic",
                sunlight_deviced::diagnostic_message(record.summary, record.error_code),
            );
        }
        if record.matched_driver == 0 && record.bound_driver == 0 {
            y = self.draw_row(canvas, theme, inner, y, "Driver", "—");
        }

        if record.irq.is_some() || record.bars.iter().any(|bar| *bar != 0) {
            y = self.draw_section(canvas, theme, inner, y + 8, "Resources", self.icons.storage);
            if let Some(irq) = record.irq {
                y = self.draw_row(canvas, theme, inner, y, "IRQ", &format!("{irq}"));
            }
            for (index, bar) in record.bars.iter().copied().enumerate() {
                if bar != 0 {
                    y = self.draw_row(
                        canvas,
                        theme,
                        inner,
                        y,
                        &format!("BAR{index}"),
                        &format!("{bar:#010x}"),
                    );
                }
            }
        }
        let _ = y;
    }

    fn draw_section(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        rect: Rect,
        y: i32,
        label: &str,
        icon: TgaImage,
    ) -> i32 {
        if y < rect.bottom() {
            canvas.draw_tga_icon_tinted(&icon, Rect::new(rect.x, y, 18, 18), theme.accent);
            draw_text(
                canvas,
                label,
                rect.x + 24,
                y + 1,
                &TextStyle::new(FontRole::UiRegular, theme.accent),
            );
            canvas.hbar(rect.x, y + 21, rect.w, 1, theme.border);
        }
        y + 28
    }

    fn draw_row(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        rect: Rect,
        y: i32,
        label: &str,
        value: &str,
    ) -> i32 {
        if y >= rect.y + 60 && y < rect.bottom() - 16 {
            draw_text(
                canvas,
                label,
                rect.x + 6,
                y,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
            draw_text(
                canvas,
                value,
                rect.x + 140,
                y,
                &TextStyle::new(FontRole::UiRegular, theme.text),
            );
        }
        y + 20
    }

    fn handle_tree_click(&mut self, x: i32, y: i32, rect: Rect) -> bool {
        let Some(snapshot) = self.presentation.snapshot.as_ref() else {
            return false;
        };
        let Some(tree_snapshot) = self.tree.as_ref() else {
            return false;
        };
        let model = DeviceTreeModel::new(snapshot, tree_snapshot, self.icons);
        let rows = self.tree_state.rebuild_rows(&model);
        let tree = TreeView::new(
            Panel::with_title(rect, "Device tree")
                .content_rect()
                .inset(4),
            &rows,
        )
        .with_font(&FONT_SMALL)
        .with_scroll_offset(self.tree_state.scroll_offset());
        let Some(hit) = tree.hit_test(x, y) else {
            return false;
        };
        match hit.id {
            TreeNodeId::Class(_) => {
                self.tree_state.toggle(&model, hit.id);
                self.tree_state.set_selected(None);
            }
            TreeNodeId::Device(_) => {
                self.tree_state.handle_hit(&model, hit);
            }
        }
        self.sync_presentation_from_tree();
        self.detail_scroll = 0;
        true
    }

    fn handle_tree_key(&mut self, keycode: u8) -> bool {
        let Some(snapshot) = self.presentation.snapshot.as_ref() else {
            return false;
        };
        let Some(tree_snapshot) = self.tree.as_ref() else {
            return false;
        };
        let model = DeviceTreeModel::new(snapshot, tree_snapshot, self.icons);
        let rows = self.tree_state.rebuild_rows(&model);
        let visible_rows = TreeView::new(Rect::new(0, 0, TREE_W, WIN_H - 100), &rows)
            .with_font(&FONT_SMALL)
            .visible_row_count();
        let changed = match keycode {
            KEY_UP => self.tree_state.move_selection(&rows, -1).is_some(),
            KEY_DOWN => self.tree_state.move_selection(&rows, 1).is_some(),
            KEY_LEFT => self
                .tree_state
                .collapse_or_select_parent(&model, &rows)
                .is_some(),
            KEY_RIGHT => self
                .tree_state
                .expand_or_select_first_child(&model)
                .is_some(),
            KEY_HOME => rows
                .first()
                .and_then(|row| self.tree_state.set_selected(Some(row.id)))
                .is_some(),
            KEY_END => rows
                .last()
                .and_then(|row| self.tree_state.set_selected(Some(row.id)))
                .is_some(),
            KEY_PGUP => self.tree_state.move_selection(&rows, -8).is_some(),
            KEY_PGDN => self.tree_state.move_selection(&rows, 8).is_some(),
            _ => false,
        };
        if changed {
            self.tree_state.ensure_selected_visible(&rows, visible_rows);
            self.sync_presentation_from_tree();
            self.detail_scroll = 0;
        }
        changed
    }
}

impl App for DevicesApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        let (toolbar, status, tree, details, refresh) = Self::layout();
        self.draw_toolbar(canvas, theme, toolbar, refresh);
        self.draw_tree(canvas, theme, tree);
        self.draw_details(canvas, theme, details);
        let right = if self.is_refreshing() {
            "Refreshing…"
        } else {
            "Read-only"
        };
        let left = self
            .presentation
            .refresh_error
            .as_deref()
            .unwrap_or(self.status_text.as_str());
        StatusBar::new(status, left, "", right).draw(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        let (_, _, tree_rect, _, refresh_rect) = Self::layout();
        match event {
            Event::Tick => self.advance_refresh(),
            Event::Click { x, y } => {
                if refresh_rect.contains(Point::new(x, y)) {
                    self.request_refresh();
                    true
                } else {
                    self.handle_tree_click(x, y, tree_rect)
                }
            }
            Event::MouseMove { x, y } => {
                let old_refresh = self.refresh_hovered;
                self.refresh_hovered = refresh_rect.contains(Point::new(x, y));
                let old = self.hovered;
                self.hovered = self
                    .presentation
                    .snapshot
                    .as_ref()
                    .zip(self.tree.as_ref())
                    .and_then(|(snapshot, tree_snapshot)| {
                        let model = DeviceTreeModel::new(snapshot, tree_snapshot, self.icons);
                        let rows = self.tree_state.rebuild_rows(&model);
                        TreeView::new(
                            Panel::with_title(tree_rect, "Device tree")
                                .content_rect()
                                .inset(4),
                            &rows,
                        )
                        .with_font(&FONT_SMALL)
                        .with_scroll_offset(self.tree_state.scroll_offset())
                        .hit_test(x, y)
                        .map(|hit| hit.id)
                    });
                old != self.hovered || old_refresh != self.refresh_hovered
            }
            Event::FocusChanged { focused } => {
                self.focused = focused;
                true
            }
            Event::Key('r') | Event::Key('R') => {
                self.request_refresh();
                true
            }
            Event::KeyPress {
                keycode: KEY_ESC,
                pressed: true,
                ..
            } => {
                request_close();
                true
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                ..
            } => self.handle_tree_key(keycode),
            _ => false,
        }
    }

    fn poll_timeout_ms(&self) -> u64 {
        if self.is_refreshing() {
            0
        } else {
            200
        }
    }

    fn on_ready(&mut self) -> bool {
        self.request_refresh();
        true
    }
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    let mut app = DevicesApp::new();
    let Some(mut window) = Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Sunlight Devices",
        decoration: WindowDecoration::Normal,
    }) else {
        debug_log("[DEVICES] failed to connect window\n");
        ProcessExit::exit(1)
    };
    window.run(&mut app);
    ProcessExit::exit(0);
}
