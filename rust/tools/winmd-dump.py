#!/usr/bin/env python3
"""Dump the WinRT metadata the native `uns` collector is built on.

The Rust build deliberately uses no external crates, so the interfaces of
`Windows.UI.Notifications.Management` are declared by hand in `src/uns.rs`.
Two facts cannot be guessed and are read from the metadata Windows ships in
`C:\\Windows\\System32\\WinMetadata\\*.winmd`:

* the IID of every interface (stored as `GuidAttribute`, exactly the value
  `QueryInterface` expects);
* the order of the methods, because that order *is* the order of the function
  pointers in the COM vtable.

Usage::

    python winmd-dump.py --list                     # all types
    python winmd-dump.py --grep UserNotification
    python winmd-dump.py --type IUserNotification --rust

Method slots are counted from 6, because the first six entries of every WinRT
vtable are `IUnknown` and `IInspectable`.  An interface a method list `requires`
is **not** part of that vtable (the generated SDK header states the same thing:
`IVectorView_impl : IInspectable /* requires IIterable<T> */`) - it is reached
with `QueryInterface`, so its methods are counted separately.

Only the tables of ECMA-335 II.22 are implemented; a table this script does not
know about is reported instead of being guessed.
"""

from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path

WINMETA = Path(r"C:\Windows\System32\WinMetadata")

# --------------------------------------------------------------------------- tables
# (kind, target) per column.  `kind` is a primitive width, a heap reference or
# an index; `target` is the indexed table (or the name of a coded index).
TABLES: dict[int, tuple[str, list[tuple[str, object]]]] = {
    0x00: ("Module", [("u16", None), ("string", None), ("guid", None), ("guid", None), ("guid", None)]),
    0x01: ("TypeRef", [("ResolutionScope", None), ("string", None), ("string", None)]),
    0x02: ("TypeDef", [("u32", None), ("string", None), ("string", None), ("TypeDefOrRef", None), ("index", 0x04), ("index", 0x06)]),
    0x03: ("FieldPtr", [("index", 0x04)]),
    0x04: ("Field", [("u16", None), ("string", None), ("blob", None)]),
    0x05: ("MethodPtr", [("index", 0x06)]),
    0x06: ("MethodDef", [("u32", None), ("u16", None), ("u16", None), ("string", None), ("blob", None), ("index", 0x08)]),
    0x07: ("ParamPtr", [("index", 0x08)]),
    0x08: ("Param", [("u16", None), ("u16", None), ("string", None)]),
    0x09: ("InterfaceImpl", [("index", 0x02), ("TypeDefOrRef", None)]),
    0x0A: ("MemberRef", [("MemberRefParent", None), ("string", None), ("blob", None)]),
    0x0B: ("Constant", [("u8", None), ("u8", None), ("HasConstant", None), ("blob", None)]),
    0x0C: ("CustomAttribute", [("HasCustomAttribute", None), ("CustomAttributeType", None), ("blob", None)]),
    0x0D: ("FieldMarshal", [("HasFieldMarshal", None), ("blob", None)]),
    0x0E: ("DeclSecurity", [("u16", None), ("HasDeclSecurity", None), ("blob", None)]),
    0x0F: ("ClassLayout", [("u16", None), ("u32", None), ("index", 0x02)]),
    0x10: ("FieldLayout", [("u32", None), ("index", 0x04)]),
    0x11: ("StandAloneSig", [("blob", None)]),
    0x12: ("EventMap", [("index", 0x02), ("index", 0x14)]),
    0x13: ("EventPtr", [("index", 0x14)]),
    0x14: ("Event", [("u16", None), ("string", None), ("TypeDefOrRef", None)]),
    0x15: ("PropertyMap", [("index", 0x02), ("index", 0x17)]),
    0x16: ("PropertyPtr", [("index", 0x17)]),
    0x17: ("Property", [("u16", None), ("string", None), ("blob", None)]),
    0x18: ("MethodSemantics", [("u16", None), ("index", 0x06), ("HasSemantics", None)]),
    0x19: ("MethodImpl", [("index", 0x02), ("MethodDefOrRef", None), ("MethodDefOrRef", None)]),
    0x1A: ("ModuleRef", [("string", None)]),
    0x1B: ("TypeSpec", [("blob", None)]),
    0x1C: ("ImplMap", [("u16", None), ("MemberForwarded", None), ("string", None), ("index", 0x1A)]),
    0x1D: ("FieldRVA", [("u32", None), ("index", 0x04)]),
    0x1E: ("EncLog", [("u32", None), ("u32", None)]),
    0x1F: ("EncMap", [("u32", None)]),
    0x20: ("Assembly", [("u32", None), ("u16", None), ("u16", None), ("u16", None), ("u16", None), ("u32", None), ("blob", None), ("string", None), ("string", None)]),
    0x21: ("AssemblyProcessor", [("u32", None)]),
    0x22: ("AssemblyOS", [("u32", None), ("u32", None), ("u32", None)]),
    0x23: ("AssemblyRef", [("u16", None), ("u16", None), ("u16", None), ("u16", None), ("u32", None), ("blob", None), ("string", None), ("string", None), ("blob", None)]),
    0x24: ("AssemblyRefProcessor", [("u32", None), ("index", 0x23)]),
    0x25: ("AssemblyRefOS", [("u32", None), ("u32", None), ("u32", None), ("index", 0x23)]),
    0x26: ("File", [("u32", None), ("string", None), ("blob", None)]),
    0x27: ("ExportedType", [("u32", None), ("u32", None), ("string", None), ("string", None), ("Implementation", None)]),
    0x28: ("ManifestResource", [("u32", None), ("u32", None), ("string", None), ("Implementation", None)]),
    0x29: ("NestedClass", [("index", 0x02), ("index", 0x02)]),
    0x2A: ("GenericParam", [("u16", None), ("u16", None), ("TypeOrMethodDef", None), ("string", None)]),
    0x2B: ("MethodSpec", [("MethodDefOrRef", None), ("blob", None)]),
    0x2C: ("GenericParamConstraint", [("index", 0x2A), ("TypeDefOrRef", None)]),
}

# Coded index: (tag bits, tables in tag order); 0xFF marks an unused tag.
CODED: dict[str, tuple[int, list[int]]] = {
    "TypeDefOrRef": (2, [0x02, 0x01, 0x1B]),
    "HasConstant": (2, [0x04, 0x08, 0x17]),
    # HasCustomAttribute: MethodDef, Field, TypeRef, TypeDef, Param, InterfaceImpl,
    # MemberRef, Module, DeclSecurity, Property, Event, StandAloneSig, ModuleRef,
    # TypeSpec, Assembly, AssemblyRef, File, ExportedType, ManifestResource,
    # GenericParam, GenericParamConstraint, MethodSpec.
    "HasCustomAttribute": (5, [0x06, 0x04, 0x01, 0x02, 0x08, 0x09, 0x0A, 0x00, 0x0E, 0x17, 0x14, 0x11, 0x1A, 0x1B, 0x20, 0x23, 0x26, 0x27, 0x28, 0x2A, 0x2C, 0x2B]),
    "HasFieldMarshal": (1, [0x04, 0x08]),
    "HasDeclSecurity": (2, [0x02, 0x06, 0x20]),
    "MemberRefParent": (3, [0x02, 0x01, 0x1A, 0x06, 0x1B]),
    "HasSemantics": (1, [0x14, 0x17]),
    "MethodDefOrRef": (1, [0x06, 0x0A]),
    "MemberForwarded": (1, [0x04, 0x06]),
    "Implementation": (2, [0x26, 0x23, 0x27]),
    "CustomAttributeType": (3, [0xFF, 0xFF, 0x06, 0x0A, 0xFF]),
    "ResolutionScope": (2, [0x00, 0x1A, 0x23, 0x01]),
    "TypeOrMethodDef": (1, [0x02, 0x06]),
}

ELEMENT_TYPES = {
    0x01: "void", 0x02: "bool", 0x03: "char", 0x04: "i1", 0x05: "u1",
    0x06: "i2", 0x07: "u2", 0x08: "i4", 0x09: "u4", 0x0A: "i8",
    0x0B: "u8", 0x0C: "r4", 0x0D: "r8", 0x0E: "string", 0x0F: "ptr",
    0x10: "byref", 0x11: "valuetype", 0x12: "class", 0x13: "var",
    0x14: "array", 0x15: "genericinst", 0x18: "i", 0x19: "u",
    0x1B: "fnptr", 0x1C: "object", 0x1D: "szarray", 0x1E: "mvar",
}

# A coded value is passed around as ("coded", group, table, row).
Coded = tuple


class Blob:
    """Cursor over a metadata blob, compressed integers included."""

    def __init__(self, data: bytes, start: int = 0) -> None:
        self.data = data
        self.pos = start

    def byte(self) -> int:
        value = self.data[self.pos]
        self.pos += 1
        return value

    def compressed(self) -> int:
        first = self.byte()
        if first & 0x80 == 0:
            return first
        if first & 0xC0 == 0x80:
            return ((first & 0x3F) << 8) | self.byte()
        return ((first & 0x1F) << 24) | (self.byte() << 16) | (self.byte() << 8) | self.byte()


class Metadata:
    """The `#~` table stream plus the string and blob heaps of a winmd file."""

    def __init__(self, path: Path) -> None:
        self.path = path
        data = path.read_bytes()
        root = self._root(data)
        offsets = {name: (offset, size) for name, offset, size in root}
        blob = self._root_blob
        strings_offset, _ = offsets["#Strings"]
        self.strings = blob[strings_offset:strings_offset + offsets["#Strings"][1]]
        blobs_offset, blobs_size = offsets["#Blob"]
        self.blobs = blob[blobs_offset:blobs_offset + blobs_size]
        table_name = "#~" if "#~" in offsets else "#-"
        table_offset, table_size = offsets[table_name]
        raw = blob[table_offset:table_offset + table_size]
        self.rows = self._read_tables(raw)

    # ----------------------------------------------------------------- windows
    def _root(self, data: bytes) -> list[tuple[str, int, int]]:
        """Absolute (name, offset, size) of every metadata stream."""
        if data[:2] != b"MZ":
            raise SystemExit(f"{self.path}: not a PE file")
        pe_offset = struct.unpack_from("<I", data, 0x3C)[0]
        if data[pe_offset:pe_offset + 4] != b"PE\0\0":
            raise SystemExit(f"{self.path}: no PE signature")
        sections = struct.unpack_from("<H", data, pe_offset + 6)[0]
        optional_size = struct.unpack_from("<H", data, pe_offset + 20)[0]
        optional = pe_offset + 24
        magic = struct.unpack_from("<H", data, optional)[0]
        directory = optional + (96 if magic == 0x10B else 112)
        cli_rva, _ = struct.unpack_from("<II", data, directory + 14 * 8)
        table = optional + optional_size

        def file_offset(rva: int) -> int | None:
            for index in range(sections):
                base = table + index * 40
                virtual_size, virtual_address, raw_size, raw_pointer = struct.unpack_from(
                    "<IIII", data, base + 8
                )
                if virtual_address <= rva < virtual_address + max(virtual_size, raw_size):
                    return raw_pointer + (rva - virtual_address)
            return None

        cli = file_offset(cli_rva)
        if cli is None:
            raise SystemExit(f"{self.path}: no CLI header")
        metadata_rva, metadata_size = struct.unpack_from("<II", data, cli + 8)
        start = file_offset(metadata_rva)
        if start is None:
            raise SystemExit(f"{self.path}: no metadata")
        self._root_blob = data[start:start + metadata_size]
        blob = self._root_blob
        if blob[:4] != b"BSJB":
            raise SystemExit(f"{self.path}: no metadata signature")
        version_length = struct.unpack_from("<I", blob, 12)[0]
        # STORAGEHEADER: fFlags, pad, iStreams - the streams start after it.
        position = 16 + version_length + 4
        stream_count = struct.unpack_from("<H", blob, position - 2)[0]
        result: list[tuple[str, int, int]] = []
        for _ in range(stream_count):
            offset, size = struct.unpack_from("<II", blob, position)
            position += 8
            end = blob.index(b"\0", position)
            name = blob[position:end].decode("ascii")
            position = end + 1
            position += (-position) % 4
            result.append((name, offset, size))
        self._streams = result
        return result

    # -------------------------------------------------------------------- rows
    def _read_tables(self, raw: bytes) -> dict[int, list[tuple]]:
        heap_sizes = raw[6]
        string_size = 4 if heap_sizes & 0x01 else 2
        guid_size = 4 if heap_sizes & 0x02 else 2
        blob_size = 4 if heap_sizes & 0x04 else 2
        valid = struct.unpack_from("<Q", raw, 8)[0]
        counts: dict[int, int] = {}
        position = 24
        for index in range(64):
            if valid >> index & 1:
                counts[index] = struct.unpack_from("<I", raw, position)[0]
                position += 4
        self.counts = counts
        unknown = [index for index in counts if index not in TABLES]
        if unknown:
            raise SystemExit(
                f"{self.path}: unknown metadata tables {unknown} "
                f"(rows {[counts[index] for index in unknown]})"
            )

        def index_size(target: int) -> int:
            return 4 if counts.get(target, 0) >= 0x10000 else 2

        def coded_size(name: str) -> int:
            bits, tables = CODED[name]
            limit = 1 << (16 - bits)
            wide = any(counts.get(t, 0) >= limit for t in tables if t != 0xFF)
            return 4 if wide else 2

        def width(kind: str, target: object) -> int:
            if kind == "u8":
                return 1
            if kind == "u16":
                return 2
            if kind == "u32":
                return 4
            if kind == "string":
                return string_size
            if kind == "guid":
                return guid_size
            if kind == "blob":
                return blob_size
            if kind == "index":
                return index_size(int(target))  # type: ignore[arg-type]
            if kind in CODED:
                return coded_size(kind)
            raise SystemExit(f"unknown column kind {kind}")

        sizes = {
            index: sum(width(kind, target) for kind, target in spec[1])
            for index, spec in TABLES.items()
        }
        offsets: dict[int, int] = {}
        for index in sorted(counts):
            offsets[index] = position
            position += sizes[index] * counts[index]
        rows: dict[int, list[tuple]] = {}
        for index in sorted(counts):
            spec = TABLES[index][1]
            result = []
            for row in range(counts[index]):
                base = offsets[index] + row * sizes[index]
                cursor = base
                values: list[object] = []
                for kind, target in spec:
                    size = width(kind, target)
                    chunk = raw[cursor:cursor + size]
                    cursor += size
                    value = int.from_bytes(chunk, "little")
                    if kind in CODED:
                        bits, tables = CODED[kind]
                        tag = value & ((1 << bits) - 1)
                        values.append(("coded", kind, tables[tag] if tag < len(tables) else 0xFF, value >> bits))
                    else:
                        values.append(value)
                result.append(tuple(values))
            rows[index] = result
        return rows

    # ------------------------------------------------------------------- heaps
    def string(self, index: int) -> str:
        if not index:
            return ""
        end = self.strings.index(b"\0", index)
        return self.strings[index:end].decode("utf-8", "replace")

    def blob(self, index: int) -> bytes:
        if not index:
            return b""
        cursor = Blob(self.blobs, index)
        length = cursor.compressed()
        return self.blobs[cursor.pos:cursor.pos + length]

    # -------------------------------------------------------------------- types
    def type_def_name(self, row: int) -> str:
        namespace = self.string(self.rows[0x02][row - 1][2])
        name = self.string(self.rows[0x02][row - 1][1])
        return f"{namespace}.{name}".strip(".")

    def type_name(self, coded: tuple) -> str:
        table, row = coded[2], coded[3]
        if table == 0x02 and 1 <= row <= self.counts.get(0x02, 0):
            return self.type_def_name(row)
        if table == 0x01 and 1 <= row <= self.counts.get(0x01, 0):
            namespace = self.string(self.rows[0x01][row - 1][2])
            name = self.string(self.rows[0x01][row - 1][1])
            return f"{namespace}.{name}".strip(".")
        if table == 0x1B:
            return "typespec:" + self.blob(self.rows[0x1B][row - 1][0]).hex()
        return f"table{table}:{row}"

    def type_row(self, name: str) -> int:
        for row in range(1, self.counts.get(0x02, 0) + 1):
            if self.type_def_name(row) == name:
                return row
        raise SystemExit(f"{self.path}: type {name} not found")

    def method_rows(self, type_row: int) -> list[int]:
        start = self.rows[0x02][type_row - 1][5]
        end = (
            self.rows[0x02][type_row][5]
            if type_row < self.counts.get(0x02, 0)
            else self.counts.get(0x06, 0) + 1
        )
        return [row for row in range(start, end) if 1 <= row <= self.counts.get(0x06, 0)]

    def method_name(self, method_row: int) -> str:
        return self.string(self.rows[0x06][method_row - 1][3])

    def interfaces_of(self, type_row: int) -> list[str]:
        return [
            self.type_name(entry[1]) for entry in self.rows.get(0x09, []) if entry[0] == type_row
        ]

    def properties_of(self, type_row: int) -> list[str]:
        maps = self.rows.get(0x15, [])
        for index, (parent, start) in enumerate(maps):
            if parent != type_row:
                continue
            end = maps[index + 1][1] if index + 1 < len(maps) else self.counts.get(0x17, 0) + 1
            return [self.string(self.rows[0x17][row - 1][1]) for row in range(start, end)]
        return []

    def attribute_name(self, coded: tuple) -> str:
        table, row = coded[2], coded[3]
        if table == 0x0A and 1 <= row <= self.counts.get(0x0A, 0):
            parent, name = self.rows[0x0A][row - 1][0], self.string(self.rows[0x0A][row - 1][1])
            return f"{self.type_name(parent)}..{name}" if name == ".ctor" else name
        if table == 0x06 and 1 <= row <= self.counts.get(0x06, 0):
            return self.method_name(row)
        return f"table{table}:{row}"

    def guid_of(self, type_row: int) -> str | None:
        """`GuidAttribute` of a TypeDef, formatted like the SDK headers do."""
        for parent, kind, value in self.rows.get(0x0C, []):
            if parent[2] != 0x02 or parent[3] != type_row:
                continue
            if not self.attribute_name(kind).startswith("Windows.Foundation.Metadata.GuidAttribute"):
                continue
            blob = self.blob(value)
            # Attribute prolog (0x0001, two bytes), then the fixed arguments:
            # UInt32, UInt16, UInt16 and the last eight bytes of the GUID as they
            # appear in the SDK headers (little endian for the first three fields).
            if len(blob) < 18 or blob[0] != 0x01 or blob[1] != 0x00:
                continue
            return "{%08x-%04x-%04x-%s-%s}" % (
                int.from_bytes(blob[2:6], "little"),
                int.from_bytes(blob[6:8], "little"),
                int.from_bytes(blob[8:10], "little"),
                blob[10:12].hex(),
                blob[12:18].hex(),
            )
        return None

    def fields_of(self, type_row: int) -> list[int]:
        start = self.rows[0x02][type_row - 1][4]
        end = (
            self.rows[0x02][type_row][4]
            if type_row < self.counts.get(0x02, 0)
            else self.counts.get(0x04, 0) + 1
        )
        return [row for row in range(start, end) if 1 <= row <= self.counts.get(0x04, 0)]

    def enum_values(self, type_row: int) -> list[tuple[str, int]]:
        """`(name, value)` of an enumeration: its fields that carry a Constant."""
        constants = {
            entry[2][3]: entry[3] for entry in self.rows.get(0x0B, []) if entry[2][2] == 0x04
        }
        result = []
        for field in self.fields_of(type_row):
            name = self.string(self.rows[0x04][field - 1][1])
            blob = self.blob(constants.get(field, 0))
            if name and blob:
                result.append((name, int.from_bytes(blob[:4], "little")))
        return result

    # -------------------------------------------------------------- signatures
    def signature(self, blob_index: int) -> str:
        blob = self.blob(blob_index)
        if not blob:
            return "(no signature)"
        try:
            return self._signature(Blob(blob))
        except (IndexError, struct.error):
            return blob.hex()

    def parameter_count(self, blob_index: int) -> int:
        blob = self.blob(blob_index)
        if not blob:
            return 0
        cursor = Blob(blob)
        calling = cursor.byte()
        if calling & 0x0F in (0x06, 0x08):  # field or property: not callable
            return 0
        if calling & 0x10:
            cursor.compressed()
        return cursor.compressed()

    def _signature(self, cursor: Blob) -> str:
        calling = cursor.byte()
        kind = calling & 0x0F
        generic = f"generic[{cursor.compressed()}] " if calling & 0x10 else ""
        if kind == 0x06:
            return generic + self._type(cursor)
        if kind == 0x00:
            count = cursor.compressed()
            result = self._type(cursor)
            arguments = ", ".join(self._type(cursor) for _ in range(count))
            return f"{generic}({arguments}) -> {result}"
        if kind == 0x08:
            count = cursor.compressed()
            result = self._type(cursor)
            arguments = ", ".join(self._type(cursor) for _ in range(count))
            return f"{generic}{result} [index: {arguments}]" if arguments else generic + result
        return f"0x{calling:02x}"

    def _type(self, cursor: Blob) -> str:
        element = cursor.byte()
        if element == 0x1D:  # SZARRAY
            return self._type(cursor) + "[]"
        if element in (0x11, 0x12):  # VALUETYPE / CLASS
            prefix = "valuetype " if element == 0x11 else "class "
            return prefix + self._typedef_or_ref(cursor)
        if element == 0x15:  # GENERICINST: class/valuetype, TypeDefOrRef, args
            cursor.byte()
            kind = self._typedef_or_ref(cursor)
            arguments = [self._type(cursor) for _ in range(cursor.compressed())]
            return f"{kind}<{', '.join(arguments)}>"
        if element == 0x13:
            return f"!{cursor.compressed()}"
        if element == 0x1E:
            return f"!!{cursor.compressed()}"
        if element == 0x10:
            return "&" + self._type(cursor)
        if element == 0x0F:
            return "*" + self._type(cursor)
        if element == 0x14:  # ARRAY
            inner = self._type(cursor)
            rank = cursor.compressed()
            for _ in range(cursor.compressed()):
                cursor.compressed()
            for _ in range(cursor.compressed()):
                cursor.compressed()
            return f"{inner}[{rank}]"
        return ELEMENT_TYPES.get(element, f"0x{element:02x}")

    def _typedef_or_ref(self, cursor: Blob) -> str:
        raw = cursor.compressed()
        table = CODED["TypeDefOrRef"][1][raw & 3]
        return self.type_name(("coded", "TypeDefOrRef", table, raw >> 2))


def type_kind(meta: Metadata, row: int) -> str:
    flags = meta.rows[0x02][row - 1][0]
    if flags & 0x20:
        return "interface"
    extends = meta.type_name(meta.rows[0x02][row - 1][3])
    if extends.endswith("System.Enum"):
        return "enum"
    if extends.endswith("System.ValueType"):
        return "struct"
    if extends.endswith("System.Delegate") or extends.endswith("System.MulticastDelegate"):
        return "delegate"
    return "class"


def describe(meta: Metadata, row: int, rust: bool) -> None:
    name = meta.type_def_name(row)
    kind = type_kind(meta, row)
    guid = meta.guid_of(row)
    print(f"\n{name}   [{kind}]")
    if guid:
        print(f"  IID {guid}")
    if kind == "interface":
        interfaces = meta.interfaces_of(row)
        print(f"  base/required: {', '.join(interfaces) if interfaces else '(IInspectable)'}")
    else:
        print(f"  extends: {meta.type_name(meta.rows[0x02][row - 1][3])}")
        interfaces = meta.interfaces_of(row)
        if interfaces:
            print(f"  implements: {', '.join(interfaces)}")
    properties = meta.properties_of(row)
    if properties:
        print(f"  properties: {', '.join(properties)}")
    if kind == "enum":
        for value_name, value in meta.enum_values(row):
            print(f"  {value_name} = {value}")
        return
    for slot, method_row in enumerate(meta.method_rows(row), start=6):
        blob = meta.rows[0x06][method_row - 1][4]
        parameters = meta.parameter_count(blob)
        print(
            f"  slot {slot:2d}  {meta.method_name(method_row)}"
            f"{meta.signature(blob)}   [{parameters} argument(s) plus the return value]"
        )
    if rust and guid:
        short = name.split(".")[-1]
        a, b, c, d, e = guid.strip("{}").split("-")
        data4 = bytes.fromhex(d + e)
        print(
            f"pub const IID_{short.upper()}: GUID = GUID {{\n"
            f"    data1: 0x{int(a, 16):08x},\n"
            f"    data2: 0x{int(b, 16):04x},\n"
            f"    data3: 0x{int(c, 16):04x},\n"
            f"    data4: [{', '.join(f'0x{byte:02x}' for byte in data4)}],\n"
            f"}};"
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--file", help="winmd file (default: Windows.UI.winmd)")
    parser.add_argument("--list", action="store_true", help="list every type")
    parser.add_argument("--grep", help="only types whose name contains this text")
    parser.add_argument("--type", action="append", default=[], help="dump this type")
    parser.add_argument("--rust", action="store_true", help="also print Rust constants")
    arguments = parser.parse_args()

    path = Path(arguments.file) if arguments.file else WINMETA / "Windows.UI.winmd"
    meta = Metadata(path)
    print(
        f"{path.name}: {meta.counts.get(0x02, 0)} types, "
        f"{meta.counts.get(0x06, 0)} methods, tables {sorted(meta.counts)}"
    )
    if arguments.list or arguments.grep:
        for row in range(1, meta.counts.get(0x02, 0) + 1):
            name = meta.type_def_name(row)
            if arguments.grep and arguments.grep.lower() not in name.lower():
                continue
            guid = meta.guid_of(row)
            print(f"  {name}  [{type_kind(meta, row)}]{'  ' + guid if guid else ''}")
    for wanted in arguments.type:
        describe(meta, meta.type_row(wanted), arguments.rust)
    return 0


if __name__ == "__main__":
    sys.exit(main())
