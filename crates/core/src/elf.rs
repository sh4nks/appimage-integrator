use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

pub type Header = [u8; 64];

const MAX_SECTION_NAMES_SIZE: u64 = 1 << 16;

struct SectionTable {
    offset: u64,
    entry_size: u64,
    count: u64,
    names_index: u64,
    is_64_bit: bool,
}

pub fn read_header(file: &mut File) -> io::Result<Option<Header>> {
    let mut header = [0u8; 64];
    match file.read_exact(&mut header) {
        Ok(()) => Ok(Some(header)),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

// The payload starts where the ELF runtime ends: right after the section header table.
pub fn payload_offset(header: &Header) -> u64 {
    let table = section_table(header);
    table.offset.saturating_add(table.entry_size * table.count)
}

pub fn section(file: &mut File, header: &Header, name: &str, limit: u64) -> io::Result<Option<Vec<u8>>> {
    let table = section_table(header);
    let (_, names_offset, names_size) = read_entry(file, &table, table.names_index)?;
    let names = read_at(file, names_offset, names_size.min(MAX_SECTION_NAMES_SIZE))?;
    for index in 0..table.count {
        let (name_offset, offset, size) = read_entry(file, &table, index)?;
        let entry_name = names.get(name_offset as usize..).and_then(|rest| rest.split(|byte| *byte == 0).next());
        if entry_name == Some(name.as_bytes()) {
            return read_at(file, offset, size.min(limit)).map(Some);
        }
    }
    Ok(None)
}

fn section_table(header: &Header) -> SectionTable {
    let u16_at = |index: usize| u64::from(u16::from_le_bytes([header[index], header[index + 1]]));
    if header[4] == 1 {
        SectionTable {
            offset: u64::from(u32_at(header, 0x20)),
            entry_size: u16_at(0x2E),
            count: u16_at(0x30),
            names_index: u16_at(0x32),
            is_64_bit: false,
        }
    } else {
        SectionTable {
            offset: u64_at(header, 0x28),
            entry_size: u16_at(0x3A),
            count: u16_at(0x3C),
            names_index: u16_at(0x3E),
            is_64_bit: true,
        }
    }
}

fn read_entry(file: &mut File, table: &SectionTable, index: u64) -> io::Result<(u32, u64, u64)> {
    let mut entry = [0u8; 64];
    let length = if table.is_64_bit { 64 } else { 40 };
    file.seek(SeekFrom::Start(table.offset.saturating_add(index * table.entry_size)))?;
    file.read_exact(&mut entry[..length])?;
    let name_offset = u32_at(&entry, 0);
    if table.is_64_bit {
        Ok((name_offset, u64_at(&entry, 0x18), u64_at(&entry, 0x20)))
    } else {
        Ok((name_offset, u64::from(u32_at(&entry, 0x10)), u64::from(u32_at(&entry, 0x14))))
    }
}

fn read_at(file: &mut File, offset: u64, length: u64) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    file.seek(SeekFrom::Start(offset))?;
    file.take(length).read_to_end(&mut data)?;
    Ok(data)
}

fn u32_at(bytes: &[u8], index: usize) -> u32 {
    u32::from_le_bytes(bytes[index..index + 4].try_into().unwrap())
}

fn u64_at(bytes: &[u8], index: usize) -> u64 {
    u64::from_le_bytes(bytes[index..index + 8].try_into().unwrap())
}
