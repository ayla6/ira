use std::cell::RefCell;
use std::rc::Rc;
use gtk4::prelude::*;
use adw::prelude::*;
use ira_models::WineConfig;

use super::wine_config_env_dll::{build_dll_override_row, collect_dll_overrides};
use super::wine_config_helpers::{build_combo_row, build_switch_row, make_section, page_with_content, track_spin, track_switch, OverrideList};
use super::css::*;

#[derive(Clone)]
pub struct WineConfigWidgets {
    pub version: String,
    pub custom_wine_path: String,
    pub arch: String,
    pub prefix: String,
    pub esync: adw::SwitchRow,
    pub fsync: adw::SwitchRow,
    pub dxvk: adw::SwitchRow,
    pub vkd3d: adw::SwitchRow,
    pub d3d_extras: adw::SwitchRow,
    pub dxvk_nvapi: adw::SwitchRow,
    pub fsr: adw::SwitchRow,
    pub battleye: adw::SwitchRow,
    pub eac: adw::SwitchRow,
    pub show_debug: adw::ComboRow,
    pub audio: adw::ComboRow,
    pub wayland: adw::SwitchRow,
    pub desktop_integration: adw::SwitchRow,
    pub show_crash_dialogs: adw::SwitchRow,
    pub mouse_warp_override: adw::ComboRow,
    pub dpi_enabled: adw::SwitchRow,
    pub dpi: gtk4::SpinButton,
    pub dxvk_frame_rate: gtk4::SpinButton,
    pub proton_wow64: adw::SwitchRow,
    pub proton_ntsync: adw::SwitchRow,
    pub dll_overrides_box: gtk4::ListBox,
    pub overridden: OverrideList,
    pub umu_enabled: bool,
}

pub struct WinePage {
    pub icon: &'static str,
    pub label: &'static str,
    pub page: gtk4::ScrolledWindow,
}

struct PerfPageWidgets {
    proton_ntsync: adw::SwitchRow,
    esync: adw::SwitchRow,
    fsync: adw::SwitchRow,
    fsr: adw::SwitchRow,
    dxvk_frame_rate: gtk4::SpinButton,
    proton_wow64: adw::SwitchRow,
}

struct GfxPageWidgets {
    dxvk: adw::SwitchRow,
    vkd3d: adw::SwitchRow,
    d3d_extras: adw::SwitchRow,
    dxvk_nvapi: adw::SwitchRow,
    wayland: adw::SwitchRow,
    mouse_warp_override: adw::ComboRow,
    dpi_enabled: adw::SwitchRow,
    dpi: gtk4::SpinButton,
    audio: adw::ComboRow,
}

struct AdvPageWidgets {
    battleye: adw::SwitchRow,
    eac: adw::SwitchRow,
    desktop_integration: adw::SwitchRow,
    show_debug: adw::ComboRow,
    show_crash_dialogs: adw::SwitchRow,
    dll_overrides_box: gtk4::ListBox,
}

fn build_wine_perf_page(
    wine: &WineConfig,
    dft: Option<&WineConfig>,
    overridden: &OverrideList,
) -> (WinePage, PerfPageWidgets) {
    let perf_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    let perf_group = make_section("Performance");

    let proton_ntsync = build_switch_row("NTSync", "Use the ntsync kernel driver for synchronization (PROTON_USE_NTSYNC)", wine.proton_ntsync);
    if let Some(dd) = dft { track_switch(&proton_ntsync, "proton_ntsync", dd.proton_ntsync, overridden); }
    perf_group.add(&proton_ntsync);
    let esync = build_switch_row("Esync", "Eventfd synchronization (requires fd limit check)", wine.esync);
    if let Some(dd) = dft { track_switch(&esync, "esync", dd.esync, overridden); }
    perf_group.add(&esync);
    let fsync = build_switch_row("Fsync", "Fast synchronization (kernel support required)", wine.fsync);
    if let Some(dd) = dft { track_switch(&fsync, "fsync", dd.fsync, overridden); }
    perf_group.add(&fsync);
    let fsr = build_switch_row("FSR", "AMD FidelityFX Super Resolution", wine.fsr);
    if let Some(dd) = dft { track_switch(&fsr, "fsr", dd.fsr, overridden); }
    perf_group.add(&fsr);

    let dxvk_frame_rate_adj = gtk4::Adjustment::new(wine.dxvk_frame_rate as f64, 0.0, 999.0, 1.0, 10.0, 0.0);
    let dxvk_frame_rate = gtk4::SpinButton::new(Some(&dxvk_frame_rate_adj), 1.0, 0);
    let dxvk_fr_row = adw::ActionRow::new();
    dxvk_fr_row.set_title("DXVK frame rate limit");
    dxvk_fr_row.set_subtitle("Sets DXVK_FRAME_RATE (0 = unlimited)");
    dxvk_frame_rate.set_valign(gtk4::Align::Center);
    dxvk_fr_row.add_suffix(&dxvk_frame_rate);
    if let Some(dd) = dft {
        track_spin(&dxvk_frame_rate, &dxvk_fr_row, "dxvk_frame_rate", dd.dxvk_frame_rate, overridden);
    }
    perf_group.add(&dxvk_fr_row);

    let proton_wow64 = build_switch_row("WoW64", "Run 32-bit Windows apps via WoW64 thunking (PROTON_USE_WOW64)", wine.proton_wow64);
    if let Some(dd) = dft { track_switch(&proton_wow64, "proton_wow64", dd.proton_wow64, overridden); }
    perf_group.add(&proton_wow64);

    perf_page.append(&perf_group);
    let page = WinePage { icon: "power-profile-performance-symbolic", label: "Wine Performance", page: page_with_content(perf_page) };

    (page, PerfPageWidgets {
        proton_ntsync, esync, fsync, fsr, dxvk_frame_rate, proton_wow64,
    })
}

fn build_wine_gfx_page(
    wine: &WineConfig,
    dft: Option<&WineConfig>,
    overridden: &OverrideList,
) -> (WinePage, GfxPageWidgets) {
    let gfx_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    let gfx_group = make_section("Graphics");

    let dxvk = build_switch_row("DXVK", "DirectX 9/10/11 to Vulkan translation", wine.dxvk);
    if let Some(dd) = dft { track_switch(&dxvk, "dxvk", dd.dxvk, overridden); }
    gfx_group.add(&dxvk);
    let vkd3d = build_switch_row("VKD3D", "DirectX 12 to Vulkan translation", wine.vkd3d);
    if let Some(dd) = dft { track_switch(&vkd3d, "vkd3d", dd.vkd3d, overridden); }
    gfx_group.add(&vkd3d);
    let d3d_extras = build_switch_row("D3D Extras", "Additional Direct3D components", wine.d3d_extras);
    if let Some(dd) = dft { track_switch(&d3d_extras, "d3d_extras", dd.d3d_extras, overridden); }
    gfx_group.add(&d3d_extras);
    let dxvk_nvapi = build_switch_row("DXVK-NVAPI / DLSS", "NVIDIA DLSS support via DXVK", wine.dxvk_nvapi);
    if let Some(dd) = dft { track_switch(&dxvk_nvapi, "dxvk_nvapi", dd.dxvk_nvapi, overridden); }
    gfx_group.add(&dxvk_nvapi);

    let wayland = build_switch_row("Enable Wayland", "Use Wayland for display instead of X11", wine.graphics == "wayland");
    if let Some(dd) = dft { track_switch(&wayland, "graphics", dd.graphics == "wayland", overridden); }
    gfx_group.add(&wayland);
    let (mouse_warp_override, _warp_model) = build_combo_row("Mouse warp override", &[("Enable", "enable"), ("Disable", "disable"), ("Force", "force")]);
    {
        let idx = match wine.mouse_warp_override.as_str() { "disable" => 1, "force" => 2, _ => 0 };
        mouse_warp_override.set_selected(idx);
    }
    gfx_group.add(&mouse_warp_override);
    let dpi_enabled = build_switch_row("Enable DPI scaling", "Override DPI settings", wine.dpi_enabled);
    if let Some(dd) = dft { track_switch(&dpi_enabled, "dpi_enabled", dd.dpi_enabled, overridden); }
    gfx_group.add(&dpi_enabled);
    let dpi_adj = gtk4::Adjustment::new(wine.dpi as f64, 96.0, 384.0, 1.0, 10.0, 0.0);
    let dpi = gtk4::SpinButton::new(Some(&dpi_adj), 1.0, 0);
    let dpi_row = adw::ActionRow::new();
    dpi_row.set_title("DPI");
    dpi_row.add_suffix(&dpi);
    if let Some(dd) = dft {
        track_spin(&dpi, &dpi_row, "dpi", dd.dpi, overridden);
    }
    dpi_row.set_visible(wine.dpi_enabled);
    gfx_group.add(&dpi_row);
    let (audio, _audio_model) = build_combo_row("Audio driver", &[("Auto", "auto"), ("ALSA", "alsa"), ("PulseAudio", "pulse"), ("OSS", "oss")]);
    {
        let idx = match wine.audio.as_str() { "alsa" => 1, "pulse" => 2, "oss" => 3, _ => 0 };
        audio.set_selected(idx);
    }
    gfx_group.add(&audio);
    gfx_page.append(&gfx_group);
    let page = WinePage { icon: "video-display-symbolic", label: "Wine Graphics", page: page_with_content(gfx_page) };

    {
        let dr = dpi_row.clone();
        dpi_enabled.connect_active_notify(move |sw| { dr.set_visible(sw.is_active()); });
    }

    (page, GfxPageWidgets {
        dxvk, vkd3d, d3d_extras, dxvk_nvapi,
        wayland, mouse_warp_override,
        dpi_enabled, dpi, audio,
    })
}

fn build_wine_adv_page(
    wine: &WineConfig,
    dft: Option<&WineConfig>,
    overridden: &OverrideList,
) -> (WinePage, AdvPageWidgets) {
    let adv_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let ac_group = make_section("Anti-Cheat");
    let battleye = build_switch_row("BattlEye", "Enable BattlEye anti-cheat support", wine.battleye);
    if let Some(dd) = dft { track_switch(&battleye, "battleye", dd.battleye, overridden); }
    ac_group.add(&battleye);
    let eac = build_switch_row("Easy Anti-Cheat", "Enable Easy Anti-Cheat support", wine.eac);
    if let Some(dd) = dft { track_switch(&eac, "eac", dd.eac, overridden); }
    ac_group.add(&eac);
    let desktop_integration = build_switch_row("Integrate system files", "Integrate desktop environment", wine.desktop_integration);
    if let Some(dd) = dft { track_switch(&desktop_integration, "desktop_integration", dd.desktop_integration, overridden); }
    ac_group.add(&desktop_integration);
    adv_page.append(&ac_group);

    let dbg_group = make_section("Debugging");
    let (show_debug, _dbg_model) = build_combo_row("Output debugging info", &[("Disabled (-all)", "-all"), ("Enabled", ""), ("Show FPS", "+fps"), ("Full (+all)", "+all")]);
    {
        let idx = match wine.show_debug.as_str() { "" => 1, "+fps" => 2, "+all" => 3, _ => 0 };
        show_debug.set_selected(idx);
    }
    dbg_group.add(&show_debug);
    let show_crash_dialogs = build_switch_row("Show crash dialogs", "Display Wine crash dialogs when programs crash", wine.show_crash_dialogs);
    if let Some(dd) = dft { track_switch(&show_crash_dialogs, "show_crash_dialogs", dd.show_crash_dialogs, overridden); }
    dbg_group.add(&show_crash_dialogs);
    adv_page.append(&dbg_group);

    let dll_group = make_section("DLL Overrides");

    let dll_overrides_box = gtk4::ListBox::new();
    dll_overrides_box.add_css_class(CSS_BOXED_LIST);
    for (name, value) in &wine.dll_overrides {
        let row = build_dll_override_row(name, value);
        dll_overrides_box.append(&row);
    }

    let add_dll_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_dll_btn.set_tooltip_text(Some("Add override"));
    add_dll_btn.set_valign(gtk4::Align::Center);
    add_dll_btn.add_css_class(CSS_FLAT);
    let box_clone = dll_overrides_box.clone();
    add_dll_btn.connect_clicked(move |_| {
        let row = build_dll_override_row("", "native,builtin");
        box_clone.append(&row);
    });
    dll_group.set_header_suffix(Some(&add_dll_btn));
    dll_group.add(&dll_overrides_box);
    adv_page.append(&dll_group);

    let page = WinePage { icon: "preferences-other-symbolic", label: "Wine Advanced", page: page_with_content(adv_page) };

    (page, AdvPageWidgets {
        battleye, eac, desktop_integration, show_debug, show_crash_dialogs,
        dll_overrides_box,
    })
}

pub fn build_wine_config_pages(wine: &WineConfig, app_default: Option<&WineConfig>) -> (Vec<WinePage>, WineConfigWidgets) {
    let mut pages = Vec::new();
    let overridden: OverrideList = Rc::new(RefCell::new(wine.overridden_fields.clone()));
    let dft = app_default;

    let (perf_page, perf_w) = build_wine_perf_page(wine, dft, &overridden);
    pages.push(perf_page);
    let (gfx_page, gfx_w) = build_wine_gfx_page(wine, dft, &overridden);
    pages.push(gfx_page);
    let (adv_page, adv_w) = build_wine_adv_page(wine, dft, &overridden);
    pages.push(adv_page);

    let widgets = WineConfigWidgets {
        version: wine.version.clone(), custom_wine_path: wine.custom_wine_path.clone(),
        arch: wine.arch.clone(), prefix: wine.prefix.clone(),
        esync: perf_w.esync, fsync: perf_w.fsync,
        dxvk: gfx_w.dxvk, vkd3d: gfx_w.vkd3d, d3d_extras: gfx_w.d3d_extras,
        dxvk_nvapi: gfx_w.dxvk_nvapi, fsr: perf_w.fsr,
        battleye: adv_w.battleye, eac: adv_w.eac,
        show_debug: adv_w.show_debug, audio: gfx_w.audio,
        wayland: gfx_w.wayland,
        desktop_integration: adv_w.desktop_integration,
        show_crash_dialogs: adv_w.show_crash_dialogs,
        mouse_warp_override: gfx_w.mouse_warp_override,
        dpi_enabled: gfx_w.dpi_enabled, dpi: gfx_w.dpi,
        dxvk_frame_rate: perf_w.dxvk_frame_rate,
        proton_wow64: perf_w.proton_wow64, proton_ntsync: perf_w.proton_ntsync,
        dll_overrides_box: adv_w.dll_overrides_box,
        overridden,
        umu_enabled: wine.umu_enabled,
    };

    (pages, widgets)
}

impl WineConfigWidgets {
    pub fn to_wine_config(&self) -> WineConfig {
        let dbg_idx = self.show_debug.selected() as usize;
        let dbg_value = match dbg_idx { 1 => "", 2 => "+fps", 3 => "+all", _ => "-all" };

        let audio_idx = self.audio.selected() as usize;
        let audio_value = match audio_idx { 1 => "alsa", 2 => "pulse", 3 => "oss", _ => "auto" };

        let gfx_value = if self.wayland.is_active() { "wayland" } else { "auto" };

        let warp_idx = self.mouse_warp_override.selected() as usize;
        let warp_value = match warp_idx { 1 => "disable", 2 => "force", _ => "enable" };

        WineConfig {
            enabled: true,
            prefix: self.prefix.clone(),
            version: self.version.clone(),
            custom_wine_path: self.custom_wine_path.clone(),
            arch: self.arch.clone(),
            esync: self.esync.is_active(),
            fsync: self.fsync.is_active(),
            dxvk: self.dxvk.is_active(),
            vkd3d: self.vkd3d.is_active(),
            d3d_extras: self.d3d_extras.is_active(),
            dxvk_nvapi: self.dxvk_nvapi.is_active(),
            fsr: self.fsr.is_active(),
            battleye: self.battleye.is_active(),
            eac: self.eac.is_active(),
            show_debug: dbg_value.to_string(),
            dll_overrides: collect_dll_overrides(&self.dll_overrides_box),
            audio: audio_value.to_string(),
            graphics: gfx_value.to_string(),
            desktop_integration: self.desktop_integration.is_active(),
            show_crash_dialogs: self.show_crash_dialogs.is_active(),
            mouse_warp_override: warp_value.to_string(),
            dpi_enabled: self.dpi_enabled.is_active(),
            dpi: self.dpi.value() as i32,
            dxvk_frame_rate: self.dxvk_frame_rate.value() as i32,
            proton_wow64: self.proton_wow64.is_active(),
            proton_ntsync: self.proton_ntsync.is_active(),
            umu_enabled: self.umu_enabled,
            overridden_fields: self.overridden.borrow().clone(),
            ..Default::default()
        }
    }
}
