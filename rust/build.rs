//! Build step: the icon, the manifest and the version information of the program.
//!
//! Windows takes the icon of a program, the manifest (look of the controls, DPI
//! awareness) and the version page of the file properties from the *resource
//! section* of the executable.  The toolchain used here is the GNU one, which has
//! no resource compiler - `windres` is not part of `rustup`, and requiring a
//! separate one would break the "no external build tools" rule of the project.
//! What such a compiler does is small and well documented, so it is done here:
//!
//! 1. `WNL.ico` is split into its images (`RT_ICON`) plus the icon group
//!    (`RT_GROUP_ICON`) that lists them with their sizes;
//! 2. `wnl.manifest` becomes `RT_MANIFEST`, the version block below `RT_VERSION`;
//! 3. everything is laid out as the resource directory Windows expects and
//!    wrapped in a minimal COFF object file with a single `.rsrc` section, which
//!    the linker turns into the resource section of the image.  The addresses of
//!    the resource bytes are RVAs, and an RVA is only known once the section is
//!    placed, so they are expressed as `ADDR32NB` relocations - the very
//!    mechanism `windres` uses.
//!
//! Nothing here is required for the program to work: a missing `WNL.ico` or an
//! unknown target architecture only means "no embedded resources", and the
//! application then falls back to the system icon and to the manifest file next
//! to the journal (`theme.rs`).

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;

/// Resource types (`RT_*` from `winuser.h`).
const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;
const RT_VERSION: u32 = 16;
const RT_MANIFEST: u32 = 24;
/// Language of every resource: English (United States).
const LANG_ENGLISH_US: u32 = 0x0409;
/// Identifier of the icon group inside the executable (`MAKEINTRESOURCE(1)`).
const ICON_GROUP_ID: u32 = 1;
/// Product name of the version page; the same words as in `appname.rs`.
const PRODUCT_NAME: &str = "Win Notification Log";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=WNL.ico");
    println!("cargo:rerun-if-changed=wnl.manifest");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let machine = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => 0x8664u16,
        Ok("x86") => 0x014cu16,
        Ok("aarch64") => 0xaa64u16,
        _ => {
            println!("cargo:warning=ресурсы не встроены: неизвестная архитектура");
            return;
        }
    };

    let mut resources = Vec::new();
    match fs::read("WNL.ico") {
        Ok(icon) => match icons(&icon) {
            Ok(found) => resources.extend(found),
            Err(error) => println!("cargo:warning=WNL.ico: {error}"),
        },
        Err(error) => println!("cargo:warning=WNL.ico не прочитан ({error}): значок не встроен"),
    }
    match fs::read("wnl.manifest") {
        Ok(text) => resources.push(Resource {
            kind: RT_MANIFEST,
            name: 1,
            data: text,
        }),
        Err(error) => println!("cargo:warning=wnl.manifest не прочитан ({error})"),
    }
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    resources.push(Resource {
        kind: RT_VERSION,
        name: 1,
        data: version_resource(&version, "wnl.exe"),
    });

    let (section, rva_fields) = resource_section(&resources);
    let file = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("wnl-resources.obj");
    if let Err(error) = fs::write(&file, coff_object(&section, &rva_fields, 0x0003, machine)) {
        println!("cargo:warning=ресурсы не записаны ({error})");
        return;
    }
    // `rustc-link-arg-bins` keeps the object out of the test binaries.
    println!("cargo:rustc-link-arg-bins={}", file.display());
}

/// One resource: its type, its identifier inside the type and the bytes.
struct Resource {
    kind: u32,
    name: u32,
    data: Vec<u8>,
}

/// A resource directory: a header followed by one entry per child.
///
/// An entry is a pair of numbers: the identifier and the offset of the child -
/// with the highest bit set when that offset points at another directory, and
/// clear when it points at the data entry of a leaf.
fn directory(entries: &[(u32, u32, bool)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0u32.to_le_bytes()); // characteristics
    out.extend_from_slice(&0u32.to_le_bytes()); // timestamp
    out.extend_from_slice(&0u16.to_le_bytes()); // major version
    out.extend_from_slice(&0u16.to_le_bytes()); // minor version
    out.extend_from_slice(&0u16.to_le_bytes()); // named entries
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (id, offset, is_directory) in entries {
        out.extend_from_slice(&id.to_le_bytes());
        let pointer = if *is_directory {
            offset | 0x8000_0000
        } else {
            *offset
        };
        out.extend_from_slice(&pointer.to_le_bytes());
    }
    out
}

/// A leaf of the resource tree: where the bytes live and how long they are.
fn data_entry(offset: u32, size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // code page
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved
    out
}

/// Lay out the whole resource section: directories, then entries, then data.
///
/// The tree has the three levels Windows expects: type, name, language.  The
/// pointers between the directories are offsets from the beginning of the
/// resource directory (which *is* the beginning of the section), but the address
/// of the bytes themselves is an RVA - and an RVA is only known once the linker
/// has placed the section.  Every data entry is therefore reported back to the
/// caller, which turns each of those addresses into a relocation (`ADDR32NB`).
///
/// The last step checks that the computed offsets and the bytes really line up:
/// a mistake would corrupt *all* resources, and a silent one at that.
fn resource_section(resources: &[Resource]) -> (Vec<u8>, Vec<u32>) {
    let mut by_type: BTreeMap<u32, BTreeMap<u32, &Resource>> = BTreeMap::new();
    for resource in resources {
        by_type
            .entry(resource.kind)
            .or_default()
            .insert(resource.name, resource);
    }

    let mut cursor = 16 + 8 * by_type.len() as u32;
    let mut type_offsets: BTreeMap<u32, u32> = BTreeMap::new();
    for (kind, names) in &by_type {
        type_offsets.insert(*kind, cursor);
        cursor += 16 + 8 * names.len() as u32;
    }
    let mut name_offsets: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for (kind, names) in &by_type {
        for name in names.keys() {
            name_offsets.insert((*kind, *name), cursor);
            cursor += 16 + 8; // exactly one language per resource
        }
    }
    let mut entry_offsets: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for (kind, names) in &by_type {
        for name in names.keys() {
            entry_offsets.insert((*kind, *name), cursor);
            cursor += 16;
        }
    }
    let mut blob_offsets: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for (kind, names) in &by_type {
        for (name, resource) in names {
            blob_offsets.insert((*kind, *name), cursor);
            cursor += (resource.data.len() as u32 + 3) & !3;
        }
    }

    let mut section = Vec::new();
    // 1. The root: one entry per type.
    let entries: Vec<(u32, u32, bool)> = by_type
        .keys()
        .map(|kind| (*kind, type_offsets[kind], true))
        .collect();
    section.extend_from_slice(&directory(&entries));
    // 2. One directory per type, listing its identifiers.
    for (kind, names) in &by_type {
        let entries: Vec<(u32, u32, bool)> = names
            .keys()
            .map(|name| (*name, name_offsets[&(*kind, *name)], true))
            .collect();
        section.extend_from_slice(&directory(&entries));
    }
    // 3. One directory per identifier: the languages.
    for (kind, names) in &by_type {
        for name in names.keys() {
            let entries = [(LANG_ENGLISH_US, entry_offsets[&(*kind, *name)], false)];
            section.extend_from_slice(&directory(&entries));
        }
    }
    // 4. The data entries, pointing at the bytes that follow them.
    let mut rva_fields = Vec::new();
    for (kind, names) in &by_type {
        for (name, resource) in names {
            rva_fields.push(section.len() as u32);
            section.extend_from_slice(&data_entry(
                blob_offsets[&(*kind, *name)],
                resource.data.len() as u32,
            ));
        }
    }
    // 5. The bytes themselves, each one padded to four bytes.
    for (kind, names) in &by_type {
        for (name, resource) in names {
            let at = blob_offsets[&(*kind, *name)];
            assert_eq!(
                section.len() as u32,
                at,
                "ресурсы разложены неверно: смещение не совпало с данными"
            );
            section.extend_from_slice(&resource.data);
            while section.len() % 4 != 0 {
                section.push(0);
            }
        }
    }
    (section, rva_fields)
}

/// Wrap the resource section in the smallest COFF object file that links.
///
/// A COFF file is a header, one section header, the section data, its
/// relocations, a symbol table and a string table.  `windres` produces exactly
/// this shape; the linker applies the relocations and turns the section into the
/// resource directory of the executable.
fn coff_object(section: &[u8], rva_fields: &[u32], relocation_kind: u16, machine: u16) -> Vec<u8> {
    let header_size = 20 + 40;
    let mut data = section.to_vec();
    while data.len() % 4 != 0 {
        data.push(0);
    }
    let relocations_at = header_size + data.len();
    let symbols_at = relocations_at + rva_fields.len() * 10;
    let mut out = Vec::new();
    // File header.
    out.extend_from_slice(&machine.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // sections
    out.extend_from_slice(&0u32.to_le_bytes()); // timestamp
    out.extend_from_slice(&(symbols_at as u32).to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes()); // one symbol: the section itself
    out.extend_from_slice(&0u16.to_le_bytes()); // optional header size
    out.extend_from_slice(&0u16.to_le_bytes()); // characteristics
    // Section header.
    out.extend_from_slice(b".rsrc\0\0\0");
    out.extend_from_slice(&0u32.to_le_bytes()); // virtual size
    out.extend_from_slice(&0u32.to_le_bytes()); // virtual address
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(header_size as u32).to_le_bytes());
    out.extend_from_slice(&(relocations_at as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // line numbers
    out.extend_from_slice(&(rva_fields.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // line number count
    // Initialized data, readable, aligned to four bytes.
    out.extend_from_slice(&0x4030_0040u32.to_le_bytes());
    out.extend_from_slice(&data);
    // Relocations: every address of the resource bytes is an RVA.
    for field in rva_fields {
        out.extend_from_slice(&field.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // symbol index: `.rsrc`
        out.extend_from_slice(&relocation_kind.to_le_bytes());
    }
    // Symbol table: one static symbol for the section, so the relocations have
    // something to refer to.
    out.extend_from_slice(b".rsrc\0\0\0");
    out.extend_from_slice(&0u32.to_le_bytes()); // value
    out.extend_from_slice(&1i16.to_le_bytes()); // section number
    out.extend_from_slice(&0u16.to_le_bytes()); // type
    out.push(3); // storage class: static
    out.push(0); // auxiliary symbols
    // String table: empty apart from its own length.
    out.extend_from_slice(&4u32.to_le_bytes());
    out
}

/// Read the images of an `.ico` file and build the matching icon group.
///
/// Icons are two resources on Windows: every image separately (`RT_ICON`) and a
/// group (`RT_GROUP_ICON`) that lists the images with the sizes Windows picks
/// from.  The group repeats the directory of the `.ico`, but where the file has
/// a byte offset the group carries the resource identifier.
fn icons(ico: &[u8]) -> Result<Vec<Resource>, String> {
    if ico.len() < 6 {
        return Err("файл короче заголовка".to_string());
    }
    let reserved = u16::from_le_bytes([ico[0], ico[1]]);
    let kind = u16::from_le_bytes([ico[2], ico[3]]);
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    if reserved != 0 || kind != 1 || count == 0 {
        return Err("это не значок (ожидался .ico)".to_string());
    }
    let mut resources = Vec::new();
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(count as u16).to_le_bytes());
    for index in 0..count {
        let entry = 6 + index * 16;
        if entry + 16 > ico.len() {
            return Err("список изображений обрывается".to_string());
        }
        let size = u32::from_le_bytes([
            ico[entry + 8],
            ico[entry + 9],
            ico[entry + 10],
            ico[entry + 11],
        ]) as usize;
        let offset = u32::from_le_bytes([
            ico[entry + 12],
            ico[entry + 13],
            ico[entry + 14],
            ico[entry + 15],
        ]) as usize;
        if offset + size > ico.len() {
            return Err("данные изображения выходят за файл".to_string());
        }
        // Size, colour depth and length, then the identifier of the image.
        group.extend_from_slice(&ico[entry..entry + 12]);
        group.extend_from_slice(&((index + 1) as u16).to_le_bytes());
        resources.push(Resource {
            kind: RT_ICON,
            name: (index + 1) as u32,
            data: ico[offset..offset + size].to_vec(),
        });
    }
    resources.push(Resource {
        kind: RT_GROUP_ICON,
        name: ICON_GROUP_ID,
        data: group,
    });
    Ok(resources)
}

/// Text as UTF-16 without the terminating zero.
fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// One block of a version resource: header, key, value, children.
///
/// `value_length` and `value_type` mean different things per level (bytes for the
/// fixed information, characters including the terminator for a string, zero for
/// a group), which is exactly how the resource is defined.
fn version_block(
    key: &str,
    value: &[u8],
    value_length: u16,
    value_type: u16,
    children: &[Vec<u8>],
) -> Vec<u8> {
    /// The header is three `WORD`s: length, value length and type.
    const HEADER: usize = 6;
    // Everything inside a block is aligned to four bytes *from the start of the
    // block* - the header is six bytes, so the padding depends on it.
    let align = |body: &mut Vec<u8>| {
        while (HEADER + body.len()) % 4 != 0 {
            body.push(0);
        }
    };
    let mut body = utf16(key);
    body.extend_from_slice(&[0, 0]);
    align(&mut body);
    body.extend_from_slice(value);
    align(&mut body);
    for child in children {
        body.extend_from_slice(child);
        align(&mut body);
    }
    let mut block = Vec::with_capacity(HEADER + body.len());
    block.extend_from_slice(&((HEADER + body.len()) as u16).to_le_bytes());
    block.extend_from_slice(&value_length.to_le_bytes());
    block.extend_from_slice(&value_type.to_le_bytes());
    block.extend_from_slice(&body);
    block
}

/// `VS_FIXEDFILEINFO`: the numbers behind "Properties → Details".
fn fixed_file_info(version: &str) -> Vec<u8> {
    let numbers: Vec<u32> = version
        .split('.')
        .filter_map(|part| part.trim().parse::<u32>().ok())
        .collect();
    let major = numbers.first().copied().unwrap_or(0);
    let minor = numbers.get(1).copied().unwrap_or(0);
    let patch = numbers.get(2).copied().unwrap_or(0);
    let build = numbers.get(3).copied().unwrap_or(0);
    let high = (major << 16) | (minor & 0xffff);
    let low = (patch << 16) | (build & 0xffff);
    let mut info = Vec::new();
    for value in [
        0xfee_f04b_du32, // signature (`VS_FFI_SIGNATURE`)
        0x0001_0000u32,  // structure version 1.0
        high,            // file version, most significant
        low,             // file version, least significant
        high,            // product version, most significant
        low,             // product version, least significant
        0x3fu32,         // file flags mask
        0u32,            // file flags
        0x0004_0004u32,  // VOS_NT_WINDOWS32
        1u32,            // VFT_APP
        0u32,            // file subtype
        0u32,            // file date, most significant
        0u32,            // file date, least significant
    ] {
        info.extend_from_slice(&value.to_le_bytes());
    }
    info
}

/// The version resource: product name, version and file name of the program.
fn version_resource(version: &str, exe: &str) -> Vec<u8> {
    let file_version = if version.is_empty() { "0.0.0" } else { version };
    let strings = [
        ("FileDescription", PRODUCT_NAME),
        ("FileVersion", file_version),
        ("InternalName", "wnl"),
        ("OriginalFilename", exe),
        ("ProductName", PRODUCT_NAME),
        ("ProductVersion", file_version),
        ("LegalCopyright", "MIT License"),
    ];
    let strings_blocks: Vec<Vec<u8>> = strings
        .iter()
        .map(|(key, value)| {
            let mut text = utf16(value);
            text.extend_from_slice(&[0, 0]);
            let words = (text.len() / 2) as u16;
            version_block(key, &text, words, 1, &[])
        })
        .collect();
    let table = version_block("040904b0", &[], 0, 1, &strings_blocks);
    let string_file_info = version_block("StringFileInfo", &[], 0, 1, &[table]);
    // Translation: language 0x0409, code page 1200 (Unicode).
    let translation = version_block("Translation", &0x04b0_0409u32.to_le_bytes(), 4, 0, &[]);
    let var_file_info = version_block("VarFileInfo", &[], 0, 1, &[translation]);
    version_block(
        "VS_VERSION_INFO",
        &fixed_file_info(file_version),
        52,
        0,
        &[string_file_info, var_file_info],
    )
}
