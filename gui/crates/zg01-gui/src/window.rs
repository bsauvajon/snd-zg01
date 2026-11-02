//! The window shell: header, sidebar, footer, pages and dialogs.
//!
//! The shell is the part of the GUI that does not depend on the device:
//! it is built once, and the application component tells it what to show
//! when the card appears or disappears. The Mixage, Micro and Avancé bodies
//! are installed by [`Shell::install_mixage`], [`Shell::install_micro`] and
//! [`Shell::install_advanced`]; the placeholder is only what the shell shows
//! before the card is read.

use gtk4::prelude::*;
use libadwaita::prelude::*;
use relm4::prelude::*;
use zg01_core::alsa::CardInfo;

use crate::app::{App, Msg, Page};
use crate::i18n::{t, tf};

/// Width of the sidebar, in logical pixels.
const SIDEBAR_WIDTH: i32 = 200;

/// The shell facts that decide which actions are usable.
///
/// Keeping this free of GTK widgets lets the sensitivity rules be tested
/// without a display: a completed Reset must re-enable Reset while the card
/// is still connected, and a disconnect must keep it disabled.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ShellState {
    /// Whether a card is available.
    connected: bool,
    /// Whether a blocking operation currently owns the card.
    busy: bool,
}

impl ShellState {
    /// Whether the Reset action can be used.
    fn reset_enabled(self) -> bool {
        self.connected && !self.busy
    }
}

/// The widgets the shell is made of.
pub(crate) struct Shell {
    /// The overlay the toasts are shown in.
    toast_overlay: adw::ToastOverlay,
    /// The window title and its subtitle.
    window_title: adw::WindowTitle,
    /// The connection dot of the header.
    status_dot: gtk::Label,
    /// The device name of the header.
    status_name: gtk::Label,
    /// The Save action.
    save_button: gtk::Button,
    /// The Reset action.
    reset_button: gtk::Button,
    /// The banner shown while the driver cache no longer describes the
    /// device, i.e. after a Reset.
    unknown_banner: adw::Banner,
    /// The banner shown when the card disappears while the app runs.
    device_banner: adw::Banner,
    /// The banner shown while startup reads or a reset runs.
    busy_banner: adw::Banner,
    /// The banner offering to reapply the last preset after a reconnect.
    reapply_banner: adw::Banner,
    /// The navigation list.
    sidebar: gtk::ListBox,
    /// The content stack: one page per screen, plus the connection view.
    stack: gtk::Stack,
    /// The connected/busy facts behind the action sensitivity.
    state: std::cell::RefCell<ShellState>,
}

impl Shell {
    /// Builds the shell and installs it on `window`.
    pub fn build(window: &adw::ApplicationWindow, sender: ComponentSender<App>) -> Self {
        let (header, window_title, status_dot, status_name, save_button, reset_button) =
            build_header(&sender);
        let (sidebar_box, sidebar) = build_sidebar(&sender);
        let stack = build_stack();
        let unknown_banner = build_unknown_banner();
        let device_banner = build_device_banner();
        let busy_banner = build_busy_banner();
        let reapply_banner = build_reapply_banner(&sender);

        let main = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        main.set_vexpand(true);
        main.append(&sidebar_box);
        main.append(&stack);

        let footer = build_footer();

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.add_top_bar(&busy_banner);
        toolbar.add_top_bar(&unknown_banner);
        toolbar.add_top_bar(&reapply_banner);
        toolbar.add_top_bar(&device_banner);
        toolbar.set_content(Some(&main));
        toolbar.add_bottom_bar(&footer);

        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&toolbar));

        window.set_content(Some(&toast_overlay));
        install_shortcuts(window, &sender);

        Shell {
            toast_overlay,
            window_title,
            status_dot,
            status_name,
            save_button,
            reset_button,
            unknown_banner,
            device_banner,
            busy_banner,
            reapply_banner,
            sidebar,
            stack,
            state: std::cell::RefCell::new(ShellState::default()),
        }
    }

    /// Shows the connected card in the header.
    pub fn show_connected(&self, card: &CardInfo) {
        self.status_dot.remove_css_class("offline");
        self.status_name.set_text(&t("ZG01"));
        self.window_title.set_subtitle(&tf(
            "Yamaha ZG01 · card {card} · {driver}",
            &[
                ("{card}", &card.index.to_string()),
                ("{driver}", &card.driver),
            ],
        ));
        {
            let mut state = self.state.borrow_mut();
            state.connected = true;
            state.busy = false;
        }
        self.refresh_reset();
        self.unknown_banner.set_revealed(false);
        self.device_banner.set_revealed(false);
        self.busy_banner.set_revealed(false);
    }

    /// Shows the connection view and the disconnected header.
    pub fn show_disconnected(&self) {
        self.status_dot.add_css_class("offline");
        self.status_name.set_text(&t("Disconnected"));
        self.window_title.set_subtitle(&t("No ZG01 card detected"));
        {
            let mut state = self.state.borrow_mut();
            state.connected = false;
            state.busy = false;
        }
        self.refresh_reset();
        self.unknown_banner.set_revealed(false);
        self.device_banner.set_revealed(false);
        self.busy_banner.set_revealed(false);
        self.reapply_banner.set_revealed(false);
        self.stack.set_visible_child_name("connection");
    }

    /// Shows the loading page while the startup read runs.
    pub fn show_loading(&self) {
        self.status_dot.remove_css_class("offline");
        self.status_name.set_text(&t("ZG01"));
        self.window_title
            .set_subtitle(&t("Reading the ZG01 controls…"));
        {
            let mut state = self.state.borrow_mut();
            state.connected = false;
            state.busy = true;
        }
        self.refresh_reset();
        self.save_button.set_sensitive(false);
        self.unknown_banner.set_revealed(false);
        self.device_banner.set_revealed(false);
        self.reapply_banner.set_revealed(false);
        self.stack.set_visible_child_name("loading");
    }

    /// Shows a banner while a blocking operation runs off the GTK thread.
    pub fn set_busy(&self, busy: bool, message: &str) {
        self.state.borrow_mut().busy = busy;
        if busy {
            self.busy_banner.set_title(message);
            self.busy_banner.set_revealed(true);
            self.save_button.set_sensitive(false);
        } else {
            self.busy_banner.set_revealed(false);
        }
        self.refresh_reset();
    }

    /// Matches the Reset action to the connected/busy state.
    fn refresh_reset(&self) {
        let state = *self.state.borrow();
        self.reset_button.set_sensitive(state.reset_enabled());
    }

    /// Offers to reapply the last preset after a reconnect.
    pub fn set_reapply(&self, visible: bool, preset: &str) {
        if visible {
            self.reapply_banner
                .set_title(&tf("Last preset: {preset}", &[("{preset}", preset)]));
            self.reapply_banner.set_revealed(true);
        } else {
            self.reapply_banner.set_revealed(false);
        }
    }

    /// Shows the card lost while the app is running.
    ///
    /// Unlike [`Shell::show_disconnected`], the current page and the last
    /// values stay visible; a banner explains that the card is gone.
    pub fn show_device_lost(&self) {
        self.status_dot.add_css_class("offline");
        self.status_name.set_text(&t("Disconnected"));
        self.window_title
            .set_subtitle(&t("ZG01 disconnected — reconnect the card"));
        self.reset_button.set_sensitive(false);
    }

    /// Shows or hides the unknown-state banner.
    ///
    /// The banner appears after a Reset: the driver cache no longer
    /// describes the device until a preset is loaded or a control is
    /// written again.
    pub fn set_unknown(&self, unknown: bool) {
        self.unknown_banner.set_revealed(unknown);
        if unknown {
            self.device_banner.set_revealed(false);
        }
    }

    /// Shows or hides the reconnect banner.
    pub fn set_device_lost(&self, lost: bool) {
        self.device_banner.set_revealed(lost);
        if lost {
            self.unknown_banner.set_revealed(false);
        }
    }

    /// Shows `page` in the content area.
    pub fn set_page(&self, page: Page) {
        self.stack.set_visible_child_name(page.stack_name());
        if let Some(row) = self.sidebar.row_at_index(page.index()) {
            self.sidebar.select_row(Some(&row));
        }
    }

    /// Installs the real Mixage body in place of its placeholder.
    ///
    /// The screen is rebuilt whenever the controls are read, so the old
    /// page is removed before the new one is added. The visible page is
    /// kept: a rebuild must not switch the user to another screen.
    pub fn install_mixage(&self, root: &impl IsA<gtk::Widget>) {
        self.install_page(Page::Mixage, root);
    }

    /// Installs the real Micro body in place of its placeholder.
    pub fn install_micro(&self, root: &impl IsA<gtk::Widget>) {
        self.install_page(Page::Micro, root);
    }

    /// Installs the real Avancé body in place of its placeholder.
    pub fn install_advanced(&self, root: &impl IsA<gtk::Widget>) {
        self.install_page(Page::Advanced, root);
    }

    /// Replaces one page of the content stack, keeping the visible page.
    fn install_page(&self, page: Page, root: &impl IsA<gtk::Widget>) {
        let name = page.stack_name();
        let was_visible = self.stack.visible_child_name().as_deref() == Some(name);
        if let Some(previous) = self.stack.child_by_name(name) {
            self.stack.remove(&previous);
        }
        self.stack.add_named(root, Some(name));
        if was_visible {
            self.stack.set_visible_child_name(name);
        }
    }

    /// Enables or disables the Save action and explains why.
    pub fn set_save_enabled(&self, enabled: bool) {
        self.save_button.set_sensitive(enabled);
        self.save_button.set_tooltip_text(Some(&if enabled {
            t("Save the settings to the ZG01")
        } else {
            t("Save unavailable: unknown values or no card")
        }));
    }

    /// Shows a toast at the bottom of the window.
    pub fn toast(&self, message: &str) {
        self.toast_overlay.add_toast(adw::Toast::new(message));
    }

    /// Opens the About dialog.
    pub fn present_about(&self, window: &adw::ApplicationWindow, card_index: Option<u32>) {
        let dialog = adw::AboutDialog::new();
        dialog.set_application_name("ZG01 Control");
        dialog.set_application_icon("audio-card-symbolic");
        dialog.set_version(env!("CARGO_PKG_VERSION"));
        dialog.set_developer_name("Brice Sauvajon");
        dialog.set_developers(&["Brice Sauvajon"]);
        dialog.set_license_type(gtk::License::Gpl20);
        dialog.set_website("https://github.com/bsauvajon/snd-zg01");
        dialog.set_issue_url("https://github.com/bsauvajon/snd-zg01/issues");
        dialog.set_copyright("© 2026 Brice Sauvajon");

        let card = card_index
            .map(|index| format!("hw:{index}"))
            .unwrap_or_else(|| t("not detected"));
        let versions = tf(
            "Driver: {driver}\nFirmware: {firmware}\nALSA card: {card}",
            &[
                ("{driver}", &driver_version()),
                ("{firmware}", &firmware_version(card_index)),
                ("{card}", &card),
            ],
        );
        dialog.set_comments(&tf(
            "Unofficial Yamaha ZG01 remote control.\n\n{versions}\n\nNot affiliated with Yamaha. ZG01 is a trademark of Yamaha.",
            &[("{versions}", &versions)],
        ));
        dialog.set_debug_info(&format!(
            "ZG01 Control {}\n{}",
            env!("CARGO_PKG_VERSION"),
            versions
        ));
        dialog.present(Some(window));
    }
}

/// Builds the header bar and returns its widgets.
fn build_header(
    sender: &ComponentSender<App>,
) -> (
    adw::HeaderBar,
    adw::WindowTitle,
    gtk::Label,
    gtk::Label,
    gtk::Button,
    gtk::Button,
) {
    let header = adw::HeaderBar::new();

    let status_dot = gtk::Label::new(Some("●"));
    status_dot.add_css_class("status-dot");
    status_dot.set_tooltip_text(Some(&t("Connection status to the ZG01")));
    let status_name = gtk::Label::new(Some(&t("ZG01")));
    status_name.add_css_class("device-name");
    let status = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    status.append(&status_dot);
    status.append(&status_name);
    header.pack_start(&status);

    let window_title = adw::WindowTitle::new(&t("ZG01 Control"), &t("Disconnected"));
    header.set_title_widget(Some(&window_title));

    let reset_button = gtk::Button::with_label(&t("Reset"));
    reset_button.add_css_class("flat");
    reset_button.set_sensitive(false);
    reset_button.set_tooltip_text(Some(&t(
        "Reloads the persisted ZG01 settings; the displayed values become unknown",
    )));
    let reset_sender = sender.clone();
    reset_button.connect_clicked(move |_| reset_sender.input(Msg::Reset));
    header.pack_end(&reset_button);

    let save_button = gtk::Button::with_label(&t("Save"));
    save_button.add_css_class("suggested-action");
    save_button.set_sensitive(false);
    let save_sender = sender.clone();
    save_button.connect_clicked(move |_| save_sender.input(Msg::Save));
    header.pack_end(&save_button);

    let about_button = gtk::Button::from_icon_name("help-about-symbolic");
    about_button.set_tooltip_text(Some(&t("About")));
    about_button.add_css_class("flat");
    let about_sender = sender.clone();
    about_button.connect_clicked(move |_| about_sender.input(Msg::About));
    header.pack_end(&about_button);

    (
        header,
        window_title,
        status_dot,
        status_name,
        save_button,
        reset_button,
    )
}

/// The banner shown when the displayed values no longer describe the
/// device.
fn build_unknown_banner() -> adw::Banner {
    let banner = adw::Banner::new(&t("Unknown state — load a preset"));
    banner.set_revealed(false);
    banner.set_tooltip_text(Some(&t(
        "The ZG01 reloaded its persisted settings: the displayed values no longer describe it.",
    )));
    banner
}

/// The banner shown when the card disappears while the app is running.
fn build_device_banner() -> adw::Banner {
    let banner = adw::Banner::new(&t("ZG01 disconnected — reconnect the card"));
    banner.set_revealed(false);
    banner.set_tooltip_text(Some(&t(
        "The last values stay displayed; reconnection is automatic.",
    )));
    banner
}

/// The banner shown while a blocking operation runs on a worker.
fn build_busy_banner() -> adw::Banner {
    let banner = adw::Banner::new(&t("Reading the ZG01 controls…"));
    banner.set_revealed(false);
    banner.set_tooltip_text(Some(&t(
        "The window stays responsive while the card is read or reset.",
    )));
    banner
}

/// The banner that offers the last preset after a reconnect.
fn build_reapply_banner(sender: &ComponentSender<App>) -> adw::Banner {
    let banner = adw::Banner::new(&t("Reapply the last preset"));
    banner.set_revealed(false);
    banner.set_button_label(Some(&t("Reapply")));
    let sender = sender.clone();
    banner.connect_button_clicked(move |_| sender.input(Msg::ReapplyPreset));
    banner
}

/// Installs the keyboard shortcuts of the window.
fn install_shortcuts(window: &adw::ApplicationWindow, sender: &ComponentSender<App>) {
    let controller = gtk::ShortcutController::new();
    controller.set_scope(gtk::ShortcutScope::Global);
    add_shortcut(&controller, "CTRL+O", sender.clone(), || Msg::OpenPreset);
    add_shortcut(&controller, "CTRL+S", sender.clone(), || Msg::Save);
    add_shortcut(&controller, "CTRL+Q", sender.clone(), || Msg::Quit);
    window.add_controller(controller);
}

/// Adds one global shortcut that sends the message built by `message`.
fn add_shortcut(
    controller: &gtk::ShortcutController,
    accelerator: &str,
    sender: ComponentSender<App>,
    message: fn() -> Msg,
) {
    let Some(trigger) = gtk::ShortcutTrigger::parse_string(accelerator) else {
        return;
    };
    let action = gtk::CallbackAction::new(move |_, _| {
        sender.input(message());
        gtk::glib::Propagation::Proceed
    });
    controller.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
}

/// Builds the sidebar and returns its container and list.
fn build_sidebar(sender: &ComponentSender<App>) -> (gtk::Box, gtk::ListBox) {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 8);
    container.add_css_class("sidebar");
    container.set_width_request(SIDEBAR_WIDTH);

    let brand = gtk::Label::new(Some(&t("ZG01 Control")));
    brand.add_css_class("sidebar-brand");
    brand.set_xalign(0.0);
    brand.set_margin_top(18);
    brand.set_margin_bottom(6);
    brand.set_margin_start(18);
    brand.set_margin_end(18);
    container.append(&brand);

    let list = gtk::ListBox::new();
    list.add_css_class("sidebar-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_valign(gtk::Align::Start);

    for page in Page::ALL {
        let row = gtk::ListBoxRow::new();
        row.set_activatable(true);

        let icon = gtk::Image::from_icon_name(page.icon());
        icon.set_pixel_size(16);
        let label = gtk::Label::new(Some(&page.title()));
        label.set_xalign(0.0);
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content.set_margin_top(8);
        content.set_margin_bottom(8);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.append(&icon);
        content.append(&label);
        row.set_child(Some(&content));
        list.append(&row);
    }

    let sender = sender.clone();
    list.connect_row_selected(move |_, row| {
        if let Some(page) = row.and_then(|row| Page::from_index(row.index())) {
            tracing::debug!("sidebar row selected: {page:?}");
            sender.input(Msg::ShowPage(page));
        }
    });

    container.append(&list);
    (container, list)
}

/// Builds the content stack.
fn build_stack() -> gtk::Stack {
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_vexpand(true);
    stack.set_hexpand(true);

    for page in Page::ALL {
        stack.add_named(&placeholder_page(page), Some(page.stack_name()));
    }
    stack.add_named(&connection_page(), Some("connection"));
    stack.add_named(&loading_page(), Some("loading"));

    stack
}

/// The page shown while startup reads the card on a worker.
fn loading_page() -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 14);
    root.set_margin_top(36);
    root.set_margin_bottom(36);
    root.set_margin_start(36);
    root.set_margin_end(36);
    root.set_valign(gtk::Align::Center);
    root.set_halign(gtk::Align::Center);

    let spinner = gtk::Spinner::new();
    spinner.set_size_request(48, 48);
    spinner.start();
    root.append(&spinner);

    let title = gtk::Label::new(Some(&t("Reading the ZG01 controls…")));
    title.add_css_class("connection-title");
    root.append(&title);

    let hint = gtk::Label::new(Some(&t(
        "The window stays responsive while the card is read.",
    )));
    hint.add_css_class("connection-hint");
    hint.set_wrap(true);
    root.append(&hint);

    root
}

/// Builds the footer with the cache badge.
fn build_footer() -> gtk::Box {
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    footer.add_css_class("footer");

    let badge = gtk::Label::new(Some(&t("Driver values (cache)")));
    badge.add_css_class("cache-badge");
    badge.set_tooltip_text(Some(&t(
        "The displayed values come from the driver cache. They are not re-read from the device on every display.",
    )));
    footer.append(&badge);

    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    footer.append(&spacer);

    let version = gtk::Label::new(Some(concat!("v", env!("CARGO_PKG_VERSION"))));
    version.add_css_class("footer-version");
    footer.append(&version);

    footer
}

/// A placeholder body for the screens later tasks fill in.
fn placeholder_page(page: Page) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.set_margin_top(24);
    root.set_margin_bottom(24);
    root.set_margin_start(24);
    root.set_margin_end(24);

    let title = gtk::Label::new(Some(&page.title()));
    title.add_css_class("page-title");
    title.set_xalign(0.0);
    root.append(&title);

    let subtitle = gtk::Label::new(Some(&page.description()));
    subtitle.add_css_class("page-subtitle");
    subtitle.set_xalign(0.0);
    subtitle.set_wrap(true);
    root.append(&subtitle);

    let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    panel.add_css_class("panel");
    panel.set_vexpand(true);

    let status = adw::StatusPage::new();
    status.set_icon_name(Some(page.icon()));
    status.set_title(&page.title());
    status.set_description(Some(&t("Screen in preparation.")));
    status.set_vexpand(true);
    panel.append(&status);

    root.append(&panel);
    root
}

/// The view shown when no ZG01 card exists.
fn connection_page() -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 14);
    root.set_margin_top(36);
    root.set_margin_bottom(36);
    root.set_margin_start(36);
    root.set_margin_end(36);
    root.set_valign(gtk::Align::Center);
    root.set_halign(gtk::Align::Center);

    let icon = gtk::Image::from_icon_name("video-display-symbolic");
    icon.set_pixel_size(64);
    root.append(&icon);

    let title = gtk::Label::new(Some(&t("ZG01 not found")));
    title.add_css_class("connection-title");
    root.append(&title);

    let hint = gtk::Label::new(Some(&t(
        "Plug the card into USB and check that the driver module is loaded.",
    )));
    hint.add_css_class("connection-hint");
    hint.set_wrap(true);
    hint.set_max_width_chars(56);
    hint.set_justify(gtk::Justification::Center);
    root.append(&hint);

    let commands = gtk::Box::new(gtk::Orientation::Vertical, 10);
    commands.add_css_class("panel");
    commands.set_margin_top(8);
    commands.append(&command_hint(
        &t("Check the USB connection"),
        "lsusb | grep 0499:1513",
    ));
    commands.append(&command_hint(
        &t("Load the driver module"),
        "sudo modprobe snd-zg01",
    ));
    commands.append(&command_hint(
        &t("Read the kernel log"),
        "journalctl -b -k --grep zg01",
    ));
    root.append(&commands);

    let retry = gtk::Label::new(Some(&t(
        "Reconnection is automatic as soon as the card appears.",
    )));
    retry.add_css_class("connection-hint");
    retry.set_wrap(true);
    retry.set_max_width_chars(56);
    retry.set_justify(gtk::Justification::Center);
    root.append(&retry);

    root
}

/// One title and one command of the connection view.
fn command_hint(title: &str, command: &str) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
    root.set_margin_top(8);
    root.set_margin_bottom(8);
    root.set_margin_start(12);
    root.set_margin_end(12);

    let title = gtk::Label::new(Some(title));
    title.add_css_class("hint-title");
    title.set_xalign(0.0);
    root.append(&title);

    let command = gtk::Label::new(Some(command));
    command.add_css_class("command");
    command.set_xalign(0.0);
    command.set_selectable(true);
    root.append(&command);

    root
}

/// The version of the loaded `snd-zg01` module.
///
/// The module does not publish `MODULE_VERSION` for now, so the sysfs
/// attribute is usually absent; `modinfo` is the fallback and reports
/// nothing either, in which case the version is unknown rather than
/// invented.
fn driver_version() -> String {
    if let Ok(text) = std::fs::read_to_string("/sys/module/snd_zg01/version") {
        let text = text.trim();
        if !text.is_empty() {
            return text.to_string();
        }
    }
    if let Ok(output) = std::process::Command::new("modinfo")
        .args(["-F", "version", "snd-zg01"])
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            let text = text.trim();
            if !text.is_empty() {
                return text.to_string();
            }
        }
    }
    t("unknown (module without a version)")
}

/// The firmware version of the card, as the USB `bcdDevice` field.
fn firmware_version(card_index: Option<u32>) -> String {
    let Some(index) = card_index else {
        return t("unavailable");
    };
    let Ok(device) = std::fs::canonicalize(format!("/sys/class/sound/card{index}/device")) else {
        return t("unreadable");
    };
    for ancestor in device.ancestors() {
        let path = ancestor.join("bcdDevice");
        if let Ok(text) = std::fs::read_to_string(path) {
            let text = text.trim();
            return format_bcd(text).unwrap_or_else(|| text.to_string());
        }
    }
    t("unreadable")
}

/// Formats a USB binary-coded-decimal version, e.g. `0150` as `1.50`.
fn format_bcd(text: &str) -> Option<String> {
    let value = u16::from_str_radix(text, 16).ok()?;
    let major = bcd_byte(value >> 8);
    let minor = bcd_byte(value & 0xff);
    Some(format!("{major}.{minor:02}"))
}

/// Reads one byte of binary-coded decimal as a number.
fn bcd_byte(byte: u16) -> u32 {
    let high = u32::from((byte >> 4) & 0xf);
    let low = u32::from(byte & 0xf);
    high * 10 + low
}

#[cfg(test)]
mod tests {
    use super::{format_bcd, ShellState};

    #[test]
    fn a_completed_reset_reenables_reset_on_a_connected_card() {
        let mut state = ShellState {
            connected: true,
            busy: false,
        };
        assert!(state.reset_enabled());

        // `reset()` shows the busy banner.
        state.busy = true;
        assert!(!state.reset_enabled(), "Reset is disabled while it runs");

        // `reset_finished()` hides the banner; the card is still there.
        state.busy = false;
        assert!(
            state.reset_enabled(),
            "a completed Reset must be repeatable without a reconnect"
        );
    }

    #[test]
    fn a_disconnected_or_busy_card_keeps_reset_disabled() {
        let disconnected = ShellState {
            connected: false,
            busy: false,
        };
        let busy = ShellState {
            connected: true,
            busy: true,
        };
        assert!(!disconnected.reset_enabled());
        assert!(!busy.reset_enabled());
    }

    #[test]
    fn formats_a_bcd_usb_version() {
        assert_eq!(format_bcd("0150").as_deref(), Some("1.50"));
        assert_eq!(format_bcd("0200").as_deref(), Some("2.00"));
        assert_eq!(format_bcd("0110").as_deref(), Some("1.10"));
    }

    #[test]
    fn rejects_a_version_that_is_not_hexadecimal() {
        assert_eq!(format_bcd(""), None);
        assert_eq!(format_bcd("zz"), None);
    }
}
