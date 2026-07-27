//! Text backend trait — abstracts pango vs cosmic-text.

use crate::ui::vertex::Vertex;
use crate::ui::widget::Size;

pub trait TextBackend {
    fn init();
    fn measure(text: &str, font_size: f32) -> Size;
    fn shape_text(
        text: &str,
        x: f32,
        y: f32,
        font_size: f32,
        color: [u8; 4],
    ) -> (Vec<Vertex>, Vec<u32>);
    fn clear_cache();
}
