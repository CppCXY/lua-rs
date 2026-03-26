pub struct FileChunkLayout {
    pub skip_offset: usize,
    pub text_start: usize,
    pub is_binary: bool,
}

pub fn inspect_file_chunk_layout(file_bytes: &[u8]) -> FileChunkLayout {
    let mut skip_offset = 0;

    if file_bytes.first() == Some(&b'#') {
        if let Some(pos) = file_bytes.iter().position(|&b| b == b'\n') {
            skip_offset = pos + 1;
        } else {
            skip_offset = file_bytes.len();
        }
    }

    if file_bytes[skip_offset..].starts_with(&[0xEF, 0xBB, 0xBF]) {
        skip_offset += 3;
    }

    FileChunkLayout {
        skip_offset,
        text_start: if file_bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            3
        } else {
            0
        },
        is_binary: file_bytes.get(skip_offset) == Some(&0x1B),
    }
}
