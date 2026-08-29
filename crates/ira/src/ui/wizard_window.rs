//! Host container shared by the add-game wizards. The auto-add flow presents
//! as a dialog inside the main window; the installer flow uses a separate
//! movable window so the library stays usable while installers run.

use adw::prelude::*;

#[derive(Clone)]
pub(super) enum WizardWindow {
    Dialog(adw::Dialog),
    Window(adw::Window),
}

impl WizardWindow {
    pub(super) fn close(&self) {
        match self {
            // AdwDialog::close reports whether the close was cancelled.
            Self::Dialog(d) => {
                d.close();
            }
            Self::Window(w) => w.close(),
        }
    }

    /// Parent widget for alerts, sub-dialogs and file choosers. Both variants
    /// host those through the plain widget interface.
    pub(super) fn as_widget(&self) -> &gtk4::Widget {
        match self {
            Self::Dialog(d) => d.upcast_ref(),
            Self::Window(w) => w.upcast_ref(),
        }
    }
}
