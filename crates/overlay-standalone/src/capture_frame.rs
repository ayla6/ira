use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::TrySendError;

use pipewire as pw;
use pw::spa;

use crate::capture_types::{CapturedFrame, WriterCommand};

pub(crate) struct PipeState {
    pub(crate) format: spa::param::video::VideoInfoRaw,
    pub(crate) screenshot: bool,
    pub(crate) recording: bool,
    pub(crate) replay: bool,
    pub(crate) writer: std::sync::mpsc::SyncSender<WriterCommand>,
}

pub(crate) fn update_format(
    state: &Rc<RefCell<PipeState>>,
    id: u32,
    param: Option<&spa::pod::Pod>,
) {
    let Some(param) = param else {
        return;
    };
    if id != spa::param::ParamType::Format.as_raw() {
        return;
    }
    let Ok((media_type, media_subtype)) = spa::param::format_utils::parse_format(param) else {
        return;
    };
    if media_type != spa::param::format::MediaType::Video
        || media_subtype != spa::param::format::MediaSubtype::Raw
    {
        return;
    }
    let mut state = state.borrow_mut();
    if let Err(error) = state.format.parse(param) {
        eprintln!("ira-overlay-standalone: failed to parse PipeWire video format: {error}");
    }
}

pub(crate) fn process_frame(stream: &pw::stream::Stream, state: &Rc<RefCell<PipeState>>) {
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let Some(data) = buffer.datas_mut().first_mut() else {
        return;
    };
    let (format, width, height, screenshot, recording, replay) = {
        let state = state.borrow();
        let size = state.format.size();
        (
            state.format.format(),
            size.width,
            size.height,
            state.screenshot,
            state.recording,
            state.replay,
        )
    };
    if (!screenshot && !recording && !replay) || width == 0 || height == 0 {
        return;
    }
    let Some(rgba) = copy_frame(data, format, width, height) else {
        eprintln!("ira-overlay-standalone: unsupported or invalid PipeWire video frame");
        return;
    };
    if screenshot {
        state.borrow_mut().screenshot = false;
    }
    let frame = CapturedFrame {
        rgba,
        width,
        height,
        screenshot,
        recording,
        replay,
    };
    let writer = state.borrow().writer.clone();
    if let Err(TrySendError::Disconnected(_)) = writer.try_send(WriterCommand::Frame(frame)) {
        eprintln!("ira-overlay-standalone: capture writer disconnected");
    }
}

fn copy_frame(
    data: &mut spa::buffer::Data,
    format: spa::param::video::VideoFormat,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let (offset, size, stride) = {
        let chunk = data.chunk();
        (
            chunk.offset() as usize,
            chunk.size() as usize,
            chunk.stride(),
        )
    };
    let bytes = data.data()?;
    convert_frame(bytes, offset, size, stride, width, height, format)
}

fn convert_frame(
    bytes: &[u8],
    offset: usize,
    size: usize,
    stride: i32,
    width: u32,
    height: u32,
    format: spa::param::video::VideoFormat,
) -> Option<Vec<u8>> {
    let row_size = (width as usize).checked_mul(4)?;
    let stride = usize::try_from(stride).ok()?;
    if stride < row_size {
        return None;
    }
    let limit = offset.checked_add(size)?;
    let output_size = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    let mut output = Vec::with_capacity(output_size);
    for row in 0..height as usize {
        let start = offset.checked_add(row.checked_mul(stride)?)?;
        let end = start.checked_add(row_size)?;
        if end > limit || end > bytes.len() {
            return None;
        }
        let row = &bytes[start..end];
        match format {
            spa::param::video::VideoFormat::RGBA => output.extend_from_slice(row),
            spa::param::video::VideoFormat::RGBx => extend_rgbx(&mut output, row),
            spa::param::video::VideoFormat::BGRx => extend_bgrx(&mut output, row),
            spa::param::video::VideoFormat::BGRA => extend_bgra(&mut output, row),
            _ => return None,
        }
    }
    Some(output)
}

fn extend_rgbx(output: &mut Vec<u8>, row: &[u8]) {
    for pixel in row.chunks_exact(4) {
        output.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
    }
}

fn extend_bgrx(output: &mut Vec<u8>, row: &[u8]) {
    for pixel in row.chunks_exact(4) {
        output.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
    }
}

fn extend_bgra(output: &mut Vec<u8>, row: &[u8]) {
    for pixel in row.chunks_exact(4) {
        output.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
}

#[cfg(test)]
mod tests {
    use super::convert_frame;
    use pipewire::spa::param::video::VideoFormat;

    #[test]
    fn test_convert_frame_bgra_to_rgba() {
        let bytes = [10, 20, 30, 40, 0, 0, 0, 0, 50, 60, 70, 80];
        let rgba = convert_frame(&bytes, 0, 12, 8, 1, 2, VideoFormat::BGRA).unwrap();
        assert_eq!(rgba, [30, 20, 10, 40, 70, 60, 50, 80]);
    }

    #[test]
    fn test_convert_frame_rejects_short_chunk() {
        assert!(convert_frame(&[0; 4], 0, 4, 4, 2, 1, VideoFormat::RGBA).is_none());
    }
}
