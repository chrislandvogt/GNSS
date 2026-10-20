//! A small writer of SQLite database files, enough for GeoPackage exports:
//! tables with their rows, and small indexes, written once.
//!
//! It follows the file format at <https://www.sqlite.org/fileformat2.html>:
//! 4096-byte pages, UTF-8 text, schema format 4, no free pages and no
//! auto-vacuum. Tables are table B-trees keyed by rowid, built bottom up,
//! with overflow pages for rows too long for a page. Indexes are single
//! pages, which is all the GeoPackage metadata tables need. Writing the file
//! directly means no SQLite library, so exports work the same natively and
//! in the browser, and the same rows always give the same bytes.

use std::cmp::Ordering;

/// The page size.
const PAGE: usize = 4096;
/// The version number stored as the writer's, as SQLite 3.46.0 would.
const SQLITE_VERSION: u32 = 3_046_000;

/// A value in a row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// A table: how it's created, and its rows by rowid.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Table {
    /// The table's name.
    pub name: String,
    /// Its CREATE TABLE statement.
    pub sql: String,
    /// Rows as (rowid, values), in increasing rowid. A column that's an
    /// INTEGER PRIMARY KEY holds NULL here: SQLite stores the rowid instead.
    pub rows: Vec<(i64, Vec<Value>)>,
    /// Indexes on the table.
    pub indexes: Vec<Index>,
}

/// An index on some of a table's columns.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Index {
    /// The index's name: `sqlite_autoindex_<table>_<n>` for the indexes
    /// SQLite makes for PRIMARY KEY and UNIQUE constraints.
    pub name: String,
    /// Its CREATE INDEX statement, or `None` for an automatic index.
    pub sql: Option<String>,
    /// The columns it covers, in order.
    pub columns: Vec<usize>,
}

/// A problem writing the database: something too big for this writer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TooBig(pub String);

/// SQLite's variable-length integer.
fn varint(value: u64, out: &mut Vec<u8>) {
    if value > 0x00ff_ffff_ffff_ffff {
        // Nine bytes: eight of seven bits, then a whole byte.
        let mut bytes = [0u8; 9];
        bytes[8] = value as u8;
        let mut v = value >> 8;
        for i in (0..8).rev() {
            bytes[i] = ((v & 0x7f) as u8) | 0x80;
            v >>= 7;
        }
        out.extend_from_slice(&bytes);
        return;
    }
    let mut bytes = Vec::with_capacity(9);
    let mut v = value;
    loop {
        bytes.push((v & 0x7f) as u8);
        v >>= 7;
        if v == 0 {
            break;
        }
    }
    bytes.reverse();
    let last = bytes.len() - 1;
    for (i, b) in bytes.iter_mut().enumerate() {
        if i < last {
            *b |= 0x80;
        }
    }
    out.extend_from_slice(&bytes);
}

fn varint_len(value: u64) -> usize {
    let mut v = Vec::new();
    varint(value, &mut v);
    v.len()
}

/// A record: SQLite's header of serial types, then the values.
fn record(values: &[Value]) -> Vec<u8> {
    let mut types = Vec::new();
    let mut body = Vec::new();
    for value in values {
        match value {
            Value::Null => varint(0, &mut types),
            Value::Integer(0) => varint(8, &mut types),
            Value::Integer(1) => varint(9, &mut types),
            Value::Integer(i) => {
                let i = *i;
                let (serial, bytes) = if i8::try_from(i).is_ok() {
                    (1, 1)
                } else if i16::try_from(i).is_ok() {
                    (2, 2)
                } else if (-(1 << 23)..(1 << 23)).contains(&i) {
                    (3, 3)
                } else if i32::try_from(i).is_ok() {
                    (4, 4)
                } else if (-(1 << 47)..(1 << 47)).contains(&i) {
                    (5, 6)
                } else {
                    (6, 8)
                };
                varint(serial, &mut types);
                body.extend_from_slice(&i.to_be_bytes()[8 - bytes..]);
            }
            Value::Real(r) => {
                varint(7, &mut types);
                body.extend_from_slice(&r.to_be_bytes());
            }
            Value::Text(t) => {
                varint(13 + 2 * t.len() as u64, &mut types);
                body.extend_from_slice(t.as_bytes());
            }
            Value::Blob(b) => {
                varint(12 + 2 * b.len() as u64, &mut types);
                body.extend_from_slice(b);
            }
        }
    }
    // The header's length includes the varint that gives it.
    let mut length = types.len() + 1;
    while varint_len(length as u64) + types.len() != length {
        length = varint_len(length as u64) + types.len();
    }
    let mut out = Vec::with_capacity(length + body.len());
    varint(length as u64, &mut out);
    out.extend_from_slice(&types);
    out.extend_from_slice(&body);
    out
}

/// SQLite's order for values in an index: NULL, then numbers, then text by
/// bytes (the BINARY collation), then blobs.
fn compare(a: &Value, b: &Value) -> Ordering {
    let rank = |v: &Value| match v {
        Value::Null => 0,
        Value::Integer(_) | Value::Real(_) => 1,
        Value::Text(_) => 2,
        Value::Blob(_) => 3,
    };
    let number = |v: &Value| match v {
        Value::Integer(i) => *i as f64,
        Value::Real(r) => *r,
        _ => 0.0,
    };
    match rank(a).cmp(&rank(b)) {
        Ordering::Equal => match (a, b) {
            (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
            (Value::Text(x), Value::Text(y)) => x.as_bytes().cmp(y.as_bytes()),
            (Value::Blob(x), Value::Blob(y)) => x.cmp(y),
            _ => number(a).total_cmp(&number(b)),
        },
        other => other,
    }
}

/// The pages being written. Page 1 is reserved for the schema's root.
struct Pages {
    pages: Vec<Vec<u8>>,
}

impl Pages {
    fn new() -> Self {
        Self {
            pages: vec![vec![0; PAGE]],
        }
    }

    /// A new page's number, counting from 1.
    fn allocate(&mut self) -> u32 {
        self.pages.push(vec![0; PAGE]);
        self.pages.len() as u32
    }

    fn page(&mut self, number: u32) -> &mut Vec<u8> {
        &mut self.pages[number as usize - 1]
    }
}

/// How much of a payload of `size` bytes a cell keeps on its page, for table
/// leaves (`index` false) or index pages (`index` true); the rest overflows.
fn local_size(size: usize, index: bool) -> usize {
    let usable = PAGE;
    let max_local = if index {
        (usable - 12) * 64 / 255 - 23
    } else {
        usable - 35
    };
    if size <= max_local {
        return size;
    }
    let min_local = (usable - 12) * 32 / 255 - 23;
    let k = min_local + (size - min_local) % (usable - 4);
    if k <= max_local { k } else { min_local }
}

/// A cell's payload part: what stays on the page, and the overflow chain's
/// first page if there is one.
fn payload(pages: &mut Pages, payload: &[u8], index: bool) -> Vec<u8> {
    let local = local_size(payload.len(), index);
    let mut out = payload[..local].to_vec();
    if local < payload.len() {
        let rest = &payload[local..];
        let chunks: Vec<&[u8]> = rest.chunks(PAGE - 4).collect();
        let numbers: Vec<u32> = chunks.iter().map(|_| pages.allocate()).collect();
        for (i, chunk) in chunks.iter().enumerate() {
            let next = numbers.get(i + 1).copied().unwrap_or(0);
            let page = pages.page(numbers[i]);
            page[..4].copy_from_slice(&next.to_be_bytes());
            page[4..4 + chunk.len()].copy_from_slice(chunk);
        }
        out.extend_from_slice(&numbers[0].to_be_bytes());
    }
    out
}

/// Writes cells onto a B-tree page at `offset` (100 on page 1, else 0).
fn write_page(page: &mut [u8], offset: usize, kind: u8, cells: &[Vec<u8>], right: Option<u32>) {
    let header = if right.is_some() { 12 } else { 8 };
    let mut content = PAGE;
    let mut pointers = Vec::with_capacity(cells.len());
    for cell in cells {
        content -= cell.len();
        page[content..content + cell.len()].copy_from_slice(cell);
        pointers.push(content as u16);
    }
    page[offset] = kind;
    page[offset + 1..offset + 3].copy_from_slice(&0u16.to_be_bytes());
    page[offset + 3..offset + 5].copy_from_slice(&(cells.len() as u16).to_be_bytes());
    // Where the cells start: the end of the page if there are none. (Zero
    // would mean 65536, for 64 KiB pages.)
    page[offset + 5..offset + 7].copy_from_slice(&(content as u16).to_be_bytes());
    page[offset + 7] = 0;
    if let Some(r) = right {
        page[offset + 8..offset + 12].copy_from_slice(&r.to_be_bytes());
    }
    for (i, p) in pointers.iter().enumerate() {
        let at = offset + header + 2 * i;
        page[at..at + 2].copy_from_slice(&p.to_be_bytes());
    }
}

/// Splits cells into pages, each holding as many as fit after a header of
/// `header` bytes and `offset` bytes before it.
fn pack(cells: &[Vec<u8>], header: usize, offset: usize) -> Vec<std::ops::Range<usize>> {
    let mut groups = Vec::new();
    let mut start = 0;
    let mut used = offset + header;
    for (i, cell) in cells.iter().enumerate() {
        let need = cell.len() + 2;
        if used + need > PAGE && i > start {
            groups.push(start..i);
            start = i;
            used = offset + header;
        }
        used += need;
    }
    groups.push(start..cells.len());
    groups
}

/// Builds a table B-tree from (rowid, record) rows and returns its root
/// page. The schema's root is page 1, after the database header.
fn table_btree(pages: &mut Pages, rows: &[(i64, Vec<u8>)], on_page_one: bool) -> u32 {
    let cells: Vec<Vec<u8>> = rows
        .iter()
        .map(|(rowid, record)| {
            let mut cell = Vec::new();
            varint(record.len() as u64, &mut cell);
            varint(*rowid as u64, &mut cell);
            cell.extend_from_slice(&payload(pages, record, false));
            cell
        })
        .collect();
    let offset = if on_page_one { 100 } else { 0 };
    // One leaf, if it fits where the root goes.
    let groups = pack(&cells, 8, offset);
    if groups.len() == 1 {
        let number = if on_page_one { 1 } else { pages.allocate() };
        write_page(pages.page(number), offset, 0x0d, &cells, None);
        return number;
    }
    // Leaves, then interior levels until one page holds them all.
    let mut level: Vec<(u32, i64)> = Vec::new();
    for group in pack(&cells, 8, 0) {
        let number = pages.allocate();
        write_page(pages.page(number), 0, 0x0d, &cells[group.clone()], None);
        level.push((number, rows[group.end - 1].0));
    }
    loop {
        let interior: Vec<Vec<u8>> = level
            .iter()
            .map(|(child, key)| {
                let mut cell = child.to_be_bytes().to_vec();
                varint(*key as u64, &mut cell);
                cell
            })
            .collect();
        // The last child of each page is its right-most pointer, not a cell.
        let fits_on_root = pack(&interior[..interior.len() - 1], 12, offset).len() == 1;
        if fits_on_root {
            let number = if on_page_one { 1 } else { pages.allocate() };
            let (last, _) = level[level.len() - 1];
            write_page(
                pages.page(number),
                offset,
                0x05,
                &interior[..interior.len() - 1],
                Some(last),
            );
            return number;
        }
        // Spread the children evenly over as few pages as hold them, so no
        // page is left nearly empty. A cell is at most 13 bytes and its
        // pointer 2, so a page holds at least this many children.
        let per_page = (PAGE - 12) / 15;
        let count = level.len().div_ceil(per_page);
        let mut next = Vec::with_capacity(count);
        let mut start = 0;
        for page_index in 0..count {
            let end = (level.len() * (page_index + 1)).div_ceil(count);
            let number = pages.allocate();
            let (right, key) = level[end - 1];
            write_page(
                pages.page(number),
                0,
                0x05,
                &interior[start..end - 1],
                Some(right),
            );
            next.push((number, key));
            start = end;
        }
        level = next;
    }
}

/// Builds a one-page index B-tree and returns its page.
fn index_btree(pages: &mut Pages, entries: &[Vec<u8>]) -> Result<u32, TooBig> {
    let cells: Vec<Vec<u8>> = entries
        .iter()
        .map(|record| {
            let mut cell = Vec::new();
            varint(record.len() as u64, &mut cell);
            cell.extend_from_slice(&payload(pages, record, true));
            cell
        })
        .collect();
    if pack(&cells, 8, 0).len() > 1 {
        return Err(TooBig("an index doesn't fit on one page".into()));
    }
    let number = pages.allocate();
    write_page(pages.page(number), 0, 0x0a, &cells, None);
    Ok(number)
}

/// Writes a database holding `tables`, with the header's user version and
/// application id set.
pub(crate) fn write(
    tables: &[Table],
    user_version: u32,
    application_id: u32,
) -> Result<Vec<u8>, TooBig> {
    let mut pages = Pages::new();
    let mut schema: Vec<(i64, Vec<u8>)> = Vec::new();
    for table in tables {
        let rows: Vec<(i64, Vec<u8>)> = table
            .rows
            .iter()
            .map(|(rowid, values)| (*rowid, record(values)))
            .collect();
        if rows.windows(2).any(|w| w[0].0 >= w[1].0) {
            return Err(TooBig(format!(
                "table {}'s rowids aren't increasing",
                table.name
            )));
        }
        let root = table_btree(&mut pages, &rows, false);
        schema.push((
            schema.len() as i64 + 1,
            record(&[
                Value::Text("table".into()),
                Value::Text(table.name.clone()),
                Value::Text(table.name.clone()),
                Value::Integer(i64::from(root)),
                Value::Text(table.sql.clone()),
            ]),
        ));
        for index in &table.indexes {
            let mut entries: Vec<Vec<Value>> = table
                .rows
                .iter()
                .map(|(rowid, values)| {
                    let mut key: Vec<Value> = index
                        .columns
                        .iter()
                        .map(|&c| values.get(c).cloned().unwrap_or(Value::Null))
                        .collect();
                    key.push(Value::Integer(*rowid));
                    key
                })
                .collect();
            entries.sort_by(|a, b| {
                a.iter()
                    .zip(b)
                    .map(|(x, y)| compare(x, y))
                    .find(|o| *o != Ordering::Equal)
                    .unwrap_or(Ordering::Equal)
            });
            let records: Vec<Vec<u8>> = entries.iter().map(|e| record(e)).collect();
            let root = index_btree(&mut pages, &records)?;
            schema.push((
                schema.len() as i64 + 1,
                record(&[
                    Value::Text("index".into()),
                    Value::Text(index.name.clone()),
                    Value::Text(table.name.clone()),
                    Value::Integer(i64::from(root)),
                    index.sql.clone().map_or(Value::Null, Value::Text),
                ]),
            ));
        }
    }
    table_btree(&mut pages, &schema, true);

    let count = pages.pages.len() as u32;
    let header = &mut pages.pages[0][..100];
    header[..16].copy_from_slice(b"SQLite format 3\0");
    header[16..18].copy_from_slice(&(PAGE as u16).to_be_bytes());
    header[18] = 1;
    header[19] = 1;
    header[20] = 0;
    header[21] = 64;
    header[22] = 32;
    header[23] = 32;
    header[24..28].copy_from_slice(&1u32.to_be_bytes());
    header[28..32].copy_from_slice(&count.to_be_bytes());
    header[40..44].copy_from_slice(&1u32.to_be_bytes());
    header[44..48].copy_from_slice(&4u32.to_be_bytes());
    header[56..60].copy_from_slice(&1u32.to_be_bytes());
    header[60..64].copy_from_slice(&user_version.to_be_bytes());
    header[68..72].copy_from_slice(&application_id.to_be_bytes());
    header[92..96].copy_from_slice(&1u32.to_be_bytes());
    header[96..100].copy_from_slice(&SQLITE_VERSION.to_be_bytes());
    Ok(pages.pages.concat())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn writes_varints_as_sqlite_does() {
        let v = |x: u64| {
            let mut out = Vec::new();
            varint(x, &mut out);
            out
        };
        assert_eq!(v(0), [0]);
        assert_eq!(v(127), [0x7f]);
        assert_eq!(v(128), [0x81, 0x00]);
        assert_eq!(v(16383), [0xff, 0x7f]);
        assert_eq!(v(16384), [0x81, 0x80, 0x00]);
        assert_eq!(v(u64::MAX).len(), 9);
        assert_eq!(v((-1i64) as u64), [0xff; 9]);
    }

    #[test]
    fn writes_records() {
        // Header length 4 (itself and three types), then the values.
        assert_eq!(
            record(&[Value::Null, Value::Integer(1), Value::Text("ab".into())]),
            [4, 0, 9, 17, b'a', b'b']
        );
        assert_eq!(record(&[Value::Integer(-2)]), [2, 1, 0xfe]);
        assert_eq!(record(&[Value::Integer(300)]), [2, 2, 1, 44]);
        assert_eq!(record(&[Value::Real(1.0)])[..2], [2, 7]);
    }

    #[test]
    fn spills_long_payloads() {
        assert_eq!(local_size(100, false), 100);
        assert_eq!(local_size(4061, false), 4061);
        // Beyond the maximum, a minimum share stays, as SQLite computes it.
        assert_eq!(local_size(5000, false), 908);
        assert_eq!(local_size(1000, true), 1000);
        assert_eq!(local_size(2000, true), 489);
    }
}
