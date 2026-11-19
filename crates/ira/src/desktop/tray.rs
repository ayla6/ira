use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Show,
    Hide,
    Quit,
}

pub(super) struct IraTray {
    sender: async_channel::Sender<TrayAction>,
    pub(super) icons: Vec<(i32, i32, Vec<u8>)>,
    visible: Arc<AtomicBool>,
}

pub struct TrayHandle {
    handle: super::tray_service::Handle,
    visible: Arc<AtomicBool>,
}

impl TrayHandle {
    pub fn set_visible(&self, visible: bool) {
        self.visible.store(visible, Ordering::Relaxed);
        self.handle.update(visible);
    }
}

fn send(sender: &async_channel::Sender<TrayAction>, action: TrayAction) {
    if let Err(error) = sender.try_send(action) {
        if !sender.is_closed() {
            eprintln!("Tray action was not delivered: {error}");
        }
    }
}

fn argb(pixels: &[u8]) -> Vec<u8> {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| [pixel[3], pixel[0], pixel[1], pixel[2]])
        .collect()
}

fn icons() -> Vec<(i32, i32, Vec<u8>)> {
    const GAMEPAD: [&[u8; 24]; 16] = [
        b"........................",
        b"........................",
        b"........................",
        b".....##############.....",
        b"....#WWWWWWWWWWWWWW#....",
        b"...#WWWWWWWWWWWWWWWW#...",
        b"...#WW#WWWWWWWWW#WWW#...",
        b"..#WW###WWWWWWWWWWWWW#..",
        b"..#WWW#WWWWWWW#WWW#WW#..",
        b"..#WWWWWWWWWWWWWWWWWW#..",
        b"..#WWWWWWW####WWWWWWW#..",
        b"..#WWWWWW#....#WWWWWW#..",
        b"...######......######...",
        b"........................",
        b"........................",
        b"........................",
    ];
    let image = image::RgbaImage::from_fn(24, 24, |x, y| {
        let pixel = if (4..20).contains(&y) {
            match GAMEPAD[(y - 4) as usize][x as usize] {
                b'#' => [0, 0, 0, 255],
                b'W' => [255, 255, 255, 255],
                _ => [0, 0, 0, 0],
            }
        } else {
            [0, 0, 0, 0]
        };
        image::Rgba(pixel)
    });
    [24, 48, 64]
        .into_iter()
        .map(|size| {
            let pixels =
                image::imageops::resize(&image, size, size, image::imageops::FilterType::Nearest);
            (
                pixels.width() as i32,
                pixels.height() as i32,
                argb(pixels.as_raw()),
            )
        })
        .collect()
}

pub fn start(sender: async_channel::Sender<TrayAction>) -> Result<TrayHandle, String> {
    let tray = IraTray::new(sender);
    let visible = tray.visible.clone();
    let handle = super::tray_service::start(tray)?;
    Ok(TrayHandle { handle, visible })
}

impl IraTray {
    pub(super) fn new(sender: async_channel::Sender<TrayAction>) -> Self {
        Self {
            sender,
            icons: icons(),
            visible: Arc::new(AtomicBool::new(true)),
        }
    }
    pub(super) fn activate(&self, action: TrayAction) {
        send(&self.sender, action);
    }
    pub(super) fn menu(&self) -> [(String, TrayAction); 2] {
        let visibility = if self.visible.load(Ordering::Relaxed) {
            (crate::tr!("Hide to background"), TrayAction::Hide)
        } else {
            (crate::tr!("Show Ira"), TrayAction::Show)
        };
        [visibility, (crate::tr!("Quit"), TrayAction::Quit)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_argb_uses_network_byte_order() {
        assert_eq!(
            argb(&[10, 20, 30, 40, 1, 2, 3, 0]),
            vec![40, 10, 20, 30, 0, 1, 2, 3]
        );
        assert!(argb(&[]).is_empty());
    }

    #[test]
    fn test_tray_exposes_monochrome_icon_and_text_only_actions() {
        let (sender, receiver) = async_channel::bounded(8);
        let tray = IraTray {
            sender,
            icons: icons(),
            visible: Arc::new(AtomicBool::new(true)),
        };
        assert_eq!(tray.menu().len(), 2);
        assert_eq!(tray.icons.len(), 3);
        for (width, height, data) in &tray.icons {
            assert_eq!(data.len(), (width * height * 4) as usize);
            for pixel in data.as_chunks::<4>().0 {
                assert_eq!(pixel[1], pixel[2]);
                assert_eq!(pixel[2], pixel[3]);
                assert!(pixel[1] == 0 || pixel[1] == 255);
            }
            assert!(data.as_chunks::<4>().0.contains(&[255, 255, 255, 255]));
            assert!(data.as_chunks::<4>().0.contains(&[255, 0, 0, 0]));
        }
        for (item, action) in tray
            .menu()
            .into_iter()
            .zip([TrayAction::Hide, TrayAction::Quit])
        {
            tray.activate(item.1);
            assert_eq!(receiver.try_recv().unwrap(), action);
        }
        tray.visible.store(false, Ordering::Relaxed);
        let item = &tray.menu()[0];
        assert_eq!(item.0, crate::tr!("Show Ira"));
        tray.activate(item.1);
        assert_eq!(receiver.try_recv().unwrap(), TrayAction::Show);
    }
}
