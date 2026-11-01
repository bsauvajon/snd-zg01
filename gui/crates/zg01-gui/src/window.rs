//! The window shell: header, sidebar, footer, pages and dialogs.
//!
//! The shell is the part of the GUI that does not depend on the device:
//! it is built once, and the application component tells it what to show
//! when the card appears or disappears. The Avancé body is a placeholder
//! until its own task replaces it; the Mixage and Micro bodies are
//! installed by [`Shell::install_mixage`] and [`Shell::install_micro`].

use gtk4::prelude::*;
use libadwaita::prelude::*;
use relm4::prelude::*;
use zg01_core::alsa::CardInfo;

use crate::app::{App, Msg, Page};

/// Width of the sidebar, in logical pixels.
const SIDEBAR_WIDTH: i32 = 200;

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
    /// The navigation list.
    sidebar: gtk::ListBox,
    /// The content stack: one page per screen, plus the connection view.
    stack: gtk::Stack,
}

impl Shell {
    /// Builds the shell and installs it on `window`.
    pub fn build(window: &adw::ApplicationWindow, sender: ComponentSender<App>) -> Self {
        let (header, window_title, status_dot, status_name, save_button) = build_header(&sender);
        let (sidebar_box, sidebar) = build_sidebar(&sender);
        let stack = build_stack();

        let main = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        main.set_vexpand(true);
        main.append(&sidebar_box);
        main.append(&stack);

        let footer = build_footer();

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&main));
        toolbar.add_bottom_bar(&footer);

        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&toolbar));

        window.set_content(Some(&toast_overlay));

        Shell {
            toast_overlay,
            window_title,
            status_dot,
            status_name,
            save_button,
            sidebar,
            stack,
        }
    }

    /// Shows the connected card in the header.
    pub fn show_connected(&self, card: &CardInfo) {
        self.status_dot.remove_css_class("offline");
        self.status_name.set_text("ZG01");
        self.window_title.set_subtitle(&format!(
            "Yamaha ZG01 · carte {} · {}",
            card.index, card.driver
        ));
    }

    /// Shows the connection view and the disconnected header.
    pub fn show_disconnected(&self) {
        self.status_dot.add_css_class("offline");
        self.status_name.set_text("Déconnecté");
        self.window_title.set_subtitle("Aucune carte ZG01 détectée");
        self.stack.set_visible_child_name("connection");
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
        self.save_button.set_tooltip_text(Some(if enabled {
            "Sauvegarder les réglages sur le ZG01"
        } else {
            "Sauvegarde indisponible : valeurs inconnues ou carte absente"
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
        dialog.set_developer_name("Bernard Sauvajon");
        dialog.set_developers(&["Bernard Sauvajon"]);
        dialog.set_license_type(gtk::License::Gpl20);
        dialog.set_website("https://github.com/bsauvajon/snd-zg01");
        dialog.set_issue_url("https://github.com/bsauvajon/snd-zg01/issues");
        dialog.set_copyright("© 2026 Bernard Sauvajon");

        let card = card_index
            .map(|index| format!("hw:{index}"))
            .unwrap_or_else(|| "non détectée".to_string());
        let versions = format!(
            "Pilote : {}\nMicrologiciel : {}\nCarte ALSA : {card}",
            driver_version(),
            firmware_version(card_index)
        );
        dialog.set_comments(&format!(
            "Télécommande non officielle du Yamaha ZG01.\n\n{versions}\n\nNon affiliée à Yamaha. ZG01 est une marque de Yamaha."
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
) {
    let header = adw::HeaderBar::new();

    let status_dot = gtk::Label::new(Some("●"));
    status_dot.add_css_class("status-dot");
    status_dot.set_tooltip_text(Some("État de la connexion au ZG01"));
    let status_name = gtk::Label::new(Some("ZG01"));
    status_name.add_css_class("device-name");
    let status = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    status.append(&status_dot);
    status.append(&status_name);
    header.pack_start(&status);

    let window_title = adw::WindowTitle::new("ZG01 Control", "Déconnecté");
    header.set_title_widget(Some(&window_title));

    let save_button = gtk::Button::with_label("Sauvegarder");
    save_button.add_css_class("suggested-action");
    save_button.set_sensitive(false);
    let save_sender = sender.clone();
    save_button.connect_clicked(move |_| save_sender.input(Msg::Save));
    header.pack_end(&save_button);

    let about_button = gtk::Button::from_icon_name("help-about-symbolic");
    about_button.set_tooltip_text(Some("À propos"));
    about_button.add_css_class("flat");
    let about_sender = sender.clone();
    about_button.connect_clicked(move |_| about_sender.input(Msg::About));
    header.pack_end(&about_button);

    (header, window_title, status_dot, status_name, save_button)
}

/// Builds the sidebar and returns its container and list.
fn build_sidebar(sender: &ComponentSender<App>) -> (gtk::Box, gtk::ListBox) {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 8);
    container.add_css_class("sidebar");
    container.set_width_request(SIDEBAR_WIDTH);

    let brand = gtk::Label::new(Some("ZG01 Control"));
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
        let label = gtk::Label::new(Some(page.title()));
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

    stack
}

/// Builds the footer with the cache badge.
fn build_footer() -> gtk::Box {
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    footer.add_css_class("footer");

    let badge = gtk::Label::new(Some("Valeurs du driver (cache)"));
    badge.add_css_class("cache-badge");
    badge.set_tooltip_text(Some(
        "Les valeurs affichées proviennent du cache du pilote. \
         Elles ne sont pas relues depuis l'appareil à chaque affichage.",
    ));
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

    let title = gtk::Label::new(Some(page.title()));
    title.add_css_class("page-title");
    title.set_xalign(0.0);
    root.append(&title);

    let subtitle = gtk::Label::new(Some(page.description()));
    subtitle.add_css_class("page-subtitle");
    subtitle.set_xalign(0.0);
    subtitle.set_wrap(true);
    root.append(&subtitle);

    let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    panel.add_css_class("panel");
    panel.set_vexpand(true);

    let status = adw::StatusPage::new();
    status.set_icon_name(Some(page.icon()));
    status.set_title(page.title());
    status.set_description(Some("Écran en préparation."));
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

    let title = gtk::Label::new(Some("ZG01 introuvable"));
    title.add_css_class("connection-title");
    root.append(&title);

    let hint = gtk::Label::new(Some(
        "Branchez la carte en USB et vérifiez que le module du pilote est chargé.",
    ));
    hint.add_css_class("connection-hint");
    hint.set_wrap(true);
    hint.set_max_width_chars(56);
    hint.set_justify(gtk::Justification::Center);
    root.append(&hint);

    let commands = gtk::Box::new(gtk::Orientation::Vertical, 10);
    commands.add_css_class("panel");
    commands.set_margin_top(8);
    commands.append(&command_hint(
        "Vérifier la connexion USB",
        "lsusb | grep 0499:1513",
    ));
    commands.append(&command_hint(
        "Charger le module du pilote",
        "sudo modprobe snd-zg01",
    ));
    commands.append(&command_hint(
        "Lire le journal du noyau",
        "journalctl -b -k --grep zg01",
    ));
    root.append(&commands);

    let retry = gtk::Label::new(Some(
        "La reconnexion est automatique dès que la carte apparaît.",
    ));
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
    "inconnue (module sans version)".to_string()
}

/// The firmware version of the card, as the USB `bcdDevice` field.
fn firmware_version(card_index: Option<u32>) -> String {
    let Some(index) = card_index else {
        return "indisponible".to_string();
    };
    let Ok(device) = std::fs::canonicalize(format!("/sys/class/sound/card{index}/device")) else {
        return "illisible".to_string();
    };
    for ancestor in device.ancestors() {
        let path = ancestor.join("bcdDevice");
        if let Ok(text) = std::fs::read_to_string(path) {
            let text = text.trim();
            return format_bcd(text).unwrap_or_else(|| text.to_string());
        }
    }
    "illisible".to_string()
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
    use super::format_bcd;

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
