//! Icon containers for built apps. Both `.ico` (Windows) and `.icns`
//! (macOS) can hold PNG images as they are, so a single PNG becomes either
//! one without decoding or resizing it.

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// The width and height of a PNG image, read from its header.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[..8] != PNG_SIGNATURE || &png[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(png[20..24].try_into().ok()?);
    Some((width, height))
}

/// Wraps a PNG into a one-image `.ico` file. Windows reads PNG-compressed
/// icon images since Vista; 256 × 256 is the largest size it shows.
pub fn png_to_ico(png: &[u8]) -> Result<Vec<u8>, String> {
    let (width, height) = png_size(png).ok_or("the icon is not a PNG image")?;
    if width > 256 || height > 256 {
        return Err(format!(
            "a PNG icon may be at most 256 × 256 pixels for Windows, this one is {} × {}",
            width, height
        ));
    }
    let size = u32::try_from(png.len()).map_err(|_| "the icon is too large")?;
    let mut ico = Vec::with_capacity(22 + png.len());
    ico.extend_from_slice(&[0, 0, 1, 0, 1, 0]); // reserved, type 1 (icon), one image
    // A size byte of 0 means 256.
    ico.push(width as u8);
    ico.push(height as u8);
    ico.extend_from_slice(&[0, 0]); // no palette, reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // colour planes
    ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    ico.extend_from_slice(&size.to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes()); // offset of the image
    ico.extend_from_slice(png);
    Ok(ico)
}

/// The largest PNG-compressed image inside an `.ico` file, if any.
pub fn png_from_ico(ico: &[u8]) -> Option<Vec<u8>> {
    if ico.len() < 6 || ico[..4] != [0, 0, 1, 0] {
        return None;
    }
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    let mut best: Option<(u32, &[u8])> = None;
    for index in 0..count {
        let entry = ico.get(6 + index * 16..6 + index * 16 + 16)?;
        let size = u32::from_le_bytes(entry[8..12].try_into().ok()?) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().ok()?) as usize;
        let image = ico.get(offset..offset.checked_add(size)?)?;
        if let Some((width, _)) = png_size(image)
            && best.is_none_or(|(w, _)| width > w)
        {
            best = Some((width, image));
        }
    }
    best.map(|(_, image)| image.to_vec())
}

/// Wraps a square PNG of a size macOS knows (16 to 1024, powers of two) into
/// an `.icns` file.
pub fn png_to_icns(png: &[u8]) -> Result<Vec<u8>, String> {
    let (width, height) = png_size(png).ok_or("the icon is not a PNG image")?;
    let kind: &[u8; 4] = match (width, height) {
        (16, 16) => b"icp4",
        (32, 32) => b"icp5",
        (64, 64) => b"icp6",
        (128, 128) => b"ic07",
        (256, 256) => b"ic08",
        (512, 512) => b"ic09",
        (1024, 1024) => b"ic10",
        _ => {
            return Err(format!(
                "a macOS icon must be a square of 16, 32, 64, 128, 256, 512 or 1024 pixels, this one is {} × {}",
                width, height
            ));
        }
    };
    let entry_len = u32::try_from(png.len() + 8).map_err(|_| "the icon is too large")?;
    let total = entry_len.checked_add(8).ok_or("the icon is too large")?;
    let mut icns = Vec::with_capacity(total as usize);
    icns.extend_from_slice(b"icns");
    icns.extend_from_slice(&total.to_be_bytes());
    icns.extend_from_slice(kind);
    icns.extend_from_slice(&entry_len.to_be_bytes());
    icns.extend_from_slice(png);
    Ok(icns)
}
