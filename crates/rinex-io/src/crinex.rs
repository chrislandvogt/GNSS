//! Hatanaka-compressed ("Compact") RINEX, CRINEX 1.0 for RINEX 2 and 3.0
//! for RINEX 3 and 4, restored to RINEX a line at a time.
//!
//! The format is Hatanaka's: Hatanaka, Y. (2008), A Compression Format and
//! Tools for GNSS Observation Data, Bulletin of the Geospatial Information
//! Authority of Japan, 55, 21-30. Epoch lines and flags are stored as text
//! differences from the previous epoch, and each observation and the clock
//! offset as a difference of up to fifth order along its arc. This is our
//! own implementation of the format; its output matches the reference
//! CRX2RNX 4.1.0 byte for byte, and the tests check that on real files.

use std::io::BufRead;

use crate::RinexError;
use crate::lines::Lines;
use crate::obs::LineSource;
use crate::text::{cols, label, leading_int, trim_end};

/// The highest order of difference the format allows.
const MAX_ORDER: usize = 5;

/// One observation's (or the clock's) difference arc.
#[derive(Debug, Clone, Copy)]
struct Arc {
    /// The value and its differences: `y[order]` is the value.
    y: [i64; MAX_ORDER + 1],
    /// How far the arc has been restored; -1 before the first value.
    order: i32,
    /// The arc's difference order, or -1 for a blank field.
    arc_order: i32,
}

impl Arc {
    const BLANK: Self = Self {
        y: [0; MAX_ORDER + 1],
        order: -1,
        arc_order: -1,
    };
}

/// The layout of one RINEX major version's epoch lines.
#[derive(Clone, Copy)]
struct Layout {
    /// The first character of an epoch that starts a new arc, in the
    /// compressed file, and what it becomes.
    top_from: u8,
    top_to: u8,
    /// Columns (from 0) of the event flag, the satellite count and the
    /// satellite list.
    event: usize,
    count: usize,
    satellites: usize,
    /// The column of the space between year and month.
    offset: usize,
    /// The clock offset's decimals and width.
    clock_decimals: u32,
    clock_width: usize,
}

const RINEX2: Layout = Layout {
    top_from: b'&',
    top_to: b' ',
    event: 28,
    count: 29,
    satellites: 32,
    offset: 3,
    clock_decimals: 9,
    clock_width: 12,
};

const RINEX3: Layout = Layout {
    top_from: b'>',
    top_to: b'>',
    event: 31,
    count: 32,
    satellites: 41,
    offset: 6,
    clock_decimals: 12,
    clock_width: 15,
};

/// A Compact RINEX file, giving the RINEX file's lines.
pub(crate) struct Crinex<R> {
    input: Lines<R>,
    /// 1 or 3.
    crinex: u8,
    /// 2, 3 or 4.
    rinex: u8,
    layout: Layout,
    header_done: bool,
    /// RINEX 2: the number of observation types.
    types: usize,
    /// RINEX 3 and 4: the number of observation types for each system.
    system_types: [Option<usize>; 256],
    /// The current epoch line, restored.
    epoch_line: Vec<u8>,
    /// The previous epoch's satellites, their arcs and their flags.
    previous: Vec<[u8; 3]>,
    arcs: Vec<Vec<Arc>>,
    flags: Vec<Vec<u8>>,
    /// The clock offset's arc.
    clock: [i64; MAX_ORDER + 1],
    clock_before: [i64; MAX_ORDER + 1],
    clock_order: i32,
    clock_arc_order: i32,
    /// After an event, the next epoch must start new arcs.
    expect_new_arcs: bool,
    /// Restored lines waiting to be read, each ending in '\n'.
    out: Vec<u8>,
    at: usize,
    /// Lines given so far.
    given: u64,
    /// A scratch line.
    line: Vec<u8>,
}

impl<R: BufRead> Crinex<R> {
    /// Starts reading, given the first line, `CRINEX VERS / TYPE`.
    pub(crate) fn new(mut input: Lines<R>, first: &[u8]) -> Result<Self, RinexError> {
        let crinex = match cols(first, 0, 3) {
            b"1.0" => 1,
            b"3.0" => 3,
            other => {
                return Err(RinexError::at(
                    1,
                    format!(
                        "Compact RINEX version {:?} isn't one this reads (1.0 or 3.0)",
                        String::from_utf8_lossy(other)
                    ),
                ));
            }
        };
        let mut line = Vec::new();
        // `CRINEX PROG / DATE`, which isn't part of the RINEX file.
        if !input.read(&mut line)? {
            return Err(RinexError::at(1, "the file ends after its first line"));
        }
        Ok(Self {
            input,
            crinex,
            rinex: 0,
            layout: RINEX3,
            header_done: false,
            types: 0,
            system_types: [None; 256],
            epoch_line: Vec::new(),
            previous: Vec::new(),
            arcs: Vec::new(),
            flags: Vec::new(),
            clock: [0; MAX_ORDER + 1],
            clock_before: [0; MAX_ORDER + 1],
            clock_order: -1,
            clock_arc_order: 0,
            expect_new_arcs: false,
            out: Vec::new(),
            at: 0,
            given: 0,
            line,
        })
    }

    fn err(&self, message: impl Into<String>) -> RinexError {
        RinexError::at(self.input.line(), message)
    }

    /// The next input line, which must exist.
    fn more(&mut self, what: &str) -> Result<(), RinexError> {
        if self.input.read(&mut self.line)? {
            Ok(())
        } else {
            Err(self.err(format!("the file is cut short, before {what}")))
        }
    }

    /// Notes the observation-type counts from a header record.
    fn note_types(&mut self, line: &[u8]) {
        match label(line) {
            b"# / TYPES OF OBSERV" if cols(line, 5, 6) != b" " => {
                if let Some(n) = leading_int(cols(line, 0, 6)) {
                    self.types = usize::try_from(n).unwrap_or(0);
                }
            }
            b"SYS / # / OBS TYPES" => {
                if let Some(&system) = line.first().filter(|&&s| s != b' ') {
                    let n = leading_int(cols(line, 3, 6)).and_then(|n| usize::try_from(n).ok());
                    if let Some(slot) = self.system_types.get_mut(usize::from(system)) {
                        *slot = n;
                    }
                }
            }
            _ => {}
        }
    }

    fn emit(&mut self, line: &[u8]) {
        self.out.extend_from_slice(trim_end(line));
        self.out.push(b'\n');
    }

    /// Restores the next part of the file into `out`. False at the end.
    fn fill(&mut self) -> Result<bool, RinexError> {
        self.out.clear();
        self.at = 0;
        if !self.header_done {
            return self.header_line();
        }
        self.epoch()
    }

    /// One header line, passed through.
    fn header_line(&mut self) -> Result<bool, RinexError> {
        self.more("END OF HEADER")?;
        let line = std::mem::take(&mut self.line);
        if self.rinex == 0 {
            if label(&line) != b"RINEX VERSION / TYPE" {
                return Err(
                    self.err("the RINEX header should start here, with RINEX VERSION / TYPE")
                );
            }
            self.rinex = match cols(&line, 5, 6) {
                b"2" => 2,
                b"3" => 3,
                b"4" => 4,
                _ => return Err(self.err("the RINEX version isn't 2, 3 or 4")),
            };
            self.layout = if self.rinex == 2 { RINEX2 } else { RINEX3 };
        }
        self.note_types(&line);
        if label(&line) == b"END OF HEADER" {
            self.header_done = true;
        }
        self.emit(&line);
        self.line = line;
        Ok(true)
    }

    /// The next input line of the body, skipping CRINEX 3's escape lines.
    /// False at the end.
    fn body_line(&mut self) -> Result<bool, RinexError> {
        loop {
            if !self.input.read(&mut self.line)? {
                return Ok(false);
            }
            if self.crinex != 3 || self.line.first() != Some(&b'&') {
                return Ok(true);
            }
        }
    }

    /// Restores one epoch, or passes through one event.
    fn epoch(&mut self) -> Result<bool, RinexError> {
        let l = self.layout;
        if !self.body_line()? || self.line.first() == Some(&0x1a) {
            return Ok(false);
        }
        let mut dline = std::mem::take(&mut self.line);
        if dline.first() == Some(&l.top_from) {
            if let Some(first) = dline.first_mut() {
                *first = l.top_to;
            }
            let event = dline.get(l.event).copied().unwrap_or(0);
            if event != b'0' && event != b'1' {
                self.event(&dline)?;
                self.line = dline;
                return Ok(true);
            }
            self.epoch_line.clear();
            self.previous.clear();
            self.expect_new_arcs = false;
        } else if self.expect_new_arcs {
            return Err(self.err("after an event, the next epoch must start new arcs, but doesn't"));
        }
        repair(&mut self.epoch_line, &dline);
        self.line = dline;

        let line = &self.epoch_line;
        let at = |i: usize| line.get(l.offset + i).copied().unwrap_or(0);
        if line.first() != Some(&l.top_to)
            || line.len() < 26 + l.offset
            || at(23) != b' '
            || at(24) != b' '
            || !at(25).is_ascii_digit()
        {
            return Err(self.err("this epoch line is damaged"));
        }
        let len = trim_end(line).len();
        self.epoch_line.truncate(len);

        let count = leading_int(cols(&self.epoch_line, l.count, l.count + 3))
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| self.err("the epoch has no satellite count"))?;
        let list = cols(&self.epoch_line, l.satellites, usize::MAX);
        if list.len() < 3 * count {
            return Err(self.err(format!("the epoch lists fewer than its {count} satellites")));
        }
        let satellites: Vec<[u8; 3]> = list
            .chunks_exact(3)
            .take(count)
            .map(|c| {
                [
                    c.first().copied().unwrap_or(b' '),
                    c.get(1).copied().unwrap_or(b' '),
                    c.get(2).copied().unwrap_or(b' '),
                ]
            })
            .collect();
        let mut types = Vec::with_capacity(count);
        for sat in &satellites {
            let n = if self.rinex == 2 {
                Some(self.types)
            } else {
                self.system_types
                    .get(usize::from(sat[0]))
                    .copied()
                    .flatten()
            };
            types.push(n.ok_or_else(|| {
                self.err(format!(
                    "satellite {} is of a system the header lists no observation types for",
                    String::from_utf8_lossy(sat)
                ))
            })?);
        }
        let table: Vec<Option<usize>> = satellites
            .iter()
            .map(|s| self.previous.iter().position(|p| p == s))
            .collect();

        // The clock offset.
        self.more("the epoch's clock offset line")?;
        let clock_line = std::mem::take(&mut self.line);
        self.read_clock(&clock_line)?;

        // Each satellite's differences and flags.
        let mut arcs: Vec<Vec<Arc>> = Vec::with_capacity(count);
        let mut flag_diffs: Vec<Vec<u8>> = Vec::with_capacity(count);
        for (i, &n) in types.iter().enumerate() {
            self.more("a satellite's observations")?;
            let (sat_arcs, flag_diff) = self.differences(n, table.get(i).copied().flatten())?;
            arcs.push(sat_arcs);
            flag_diffs.push(flag_diff);
        }

        if !clock_line.is_empty() {
            self.restore_clock()?;
        }
        self.line = clock_line;
        self.write_epoch_line(count)?;
        self.write_data(&satellites, &types, &table, &mut arcs, &flag_diffs)?;

        self.previous = satellites;
        self.arcs = arcs;
        self.clock_before = self.clock;
        Ok(true)
    }

    /// An event (flags 2 to 5): its epoch line and records, as stored.
    fn event(&mut self, dline: &[u8]) -> Result<(), RinexError> {
        self.emit(dline);
        let trimmed = trim_end(dline);
        if trimmed.len() > 29 {
            let n = leading_int(cols(trimmed, self.layout.event + 1, usize::MAX)).unwrap_or(0);
            for _ in 0..n {
                self.more("the event's records")?;
                let line = std::mem::take(&mut self.line);
                self.note_types(&line);
                self.emit(&line);
                self.line = line;
            }
        }
        self.expect_new_arcs = true;
        Ok(())
    }

    /// The clock offset line: blank, a difference, or `n&value` to start an
    /// arc of order n.
    fn read_clock(&mut self, line: &[u8]) -> Result<(), RinexError> {
        if line.is_empty() {
            self.clock_order = -1;
            return Ok(());
        }
        let mut value = line;
        if line.get(1) == Some(&b'&') {
            self.clock_arc_order = order_digit(line)
                .ok_or_else(|| self.err("the clock offset's difference order isn't 0 to 5"))?;
            self.clock_order = -1;
            value = cols(line, 2, usize::MAX);
        }
        let parsed = integer(value).ok_or_else(|| {
            self.err(format!(
                "the clock offset {:?} isn't an integer",
                String::from_utf8_lossy(value)
            ))
        })?;
        if let Some(slot) = self.clock.first_mut() {
            *slot = parsed;
        }
        Ok(())
    }

    fn restore_clock(&mut self) -> Result<(), RinexError> {
        let catching_up = self.clock_order < self.clock_arc_order;
        if catching_up {
            self.clock_order += 1;
        }
        for i in 0..usize::try_from(self.clock_order).unwrap_or(0) {
            let before = if catching_up { i } else { i + 1 };
            let sum = get(&self.clock, i)
                .checked_add(get(&self.clock_before, before))
                .ok_or_else(|| self.err("the clock offset overflows"))?;
            set(&mut self.clock, i + 1, sum);
        }
        Ok(())
    }

    /// A satellite's data line: a field for each type, separated by single
    /// spaces, then the flags' text difference.
    fn differences(
        &self,
        n: usize,
        previous: Option<usize>,
    ) -> Result<(Vec<Arc>, Vec<u8>), RinexError> {
        let line = &self.line;
        let mut arcs = Vec::with_capacity(n);
        let mut pos = 0;
        let mut ended = false;
        for j in 0..n {
            let field: &[u8] = if ended {
                &[]
            } else {
                let rest = cols(line, pos, usize::MAX);
                match rest.iter().position(|&b| b == b' ') {
                    Some(k) => {
                        pos += k + 1;
                        cols(rest, 0, k)
                    }
                    None => {
                        ended = true;
                        pos = line.len() + 1;
                        rest
                    }
                }
            };
            if field.is_empty() {
                arcs.push(Arc::BLANK);
                continue;
            }
            let mut arc = Arc::BLANK;
            let mut digits = field;
            if field.get(1) == Some(&b'&') {
                arc.arc_order = order_digit(field)
                    .ok_or_else(|| self.err("an observation's difference order isn't 0 to 5"))?;
                digits = cols(field, 2, usize::MAX);
            } else {
                let before = previous.and_then(|p| self.arcs.get(p)?.get(j));
                match before {
                    None => {
                        return Err(self.err("a satellite's data starts without starting its arc"));
                    }
                    Some(b) if b.arc_order < 0 => {
                        return Err(
                            self.err("a blank observation resumes without starting a new arc")
                        );
                    }
                    Some(b) => {
                        arc.order = b.order;
                        arc.arc_order = b.arc_order;
                    }
                }
            }
            arc.y[0] = integer(digits).ok_or_else(|| {
                self.err(format!(
                    "{:?} isn't an integer difference",
                    String::from_utf8_lossy(digits)
                ))
            })?;
            arcs.push(arc);
        }
        let flags = cols(line, pos, usize::MAX).to_vec();
        Ok((arcs, flags))
    }

    /// The restored epoch line or lines, with the clock offset.
    fn write_epoch_line(&mut self, count: usize) -> Result<(), RinexError> {
        let clock = if self.clock_order >= 0 {
            let value = get(&self.clock, usize::try_from(self.clock_order).unwrap_or(0));
            Some(
                fixed(value, self.layout.clock_decimals, self.layout.clock_width)
                    .ok_or_else(|| self.err("the clock offset is too large for RINEX"))?,
            )
        } else {
            None
        };
        let line = std::mem::take(&mut self.epoch_line);
        if self.rinex == 2 {
            let head = cols(&line, 0, 68);
            match &clock {
                Some(c) => {
                    self.out.extend_from_slice(head);
                    self.out.resize(self.out.len() + 68 - head.len(), b' ');
                    self.out.extend_from_slice(c.as_bytes());
                    self.out.push(b'\n');
                }
                None => {
                    self.out.extend_from_slice(head);
                    self.out.push(b'\n');
                }
            }
            let mut start = 68;
            let mut left = count.saturating_sub(12);
            while left > 0 {
                self.out.extend_from_slice(&[b' '; 32]);
                self.out.extend_from_slice(cols(&line, start, start + 36));
                self.out.push(b'\n');
                start += 36;
                left = left.saturating_sub(12);
            }
        } else {
            let head = cols(&line, 0, 41);
            match &clock {
                Some(c) => {
                    self.out.extend_from_slice(head);
                    self.out.extend_from_slice(c.as_bytes());
                    self.out.push(b'\n');
                }
                None => self.emit(head),
            }
        }
        self.epoch_line = line;
        Ok(())
    }

    /// The restored observations: each value from its differences, with its
    /// flags.
    fn write_data(
        &mut self,
        satellites: &[[u8; 3]],
        types: &[usize],
        table: &[Option<usize>],
        arcs: &mut [Vec<Arc>],
        flag_diffs: &[Vec<u8>],
    ) -> Result<(), RinexError> {
        let mut new_flags = Vec::with_capacity(satellites.len());
        for (i, sat) in satellites.iter().enumerate() {
            let n = types.get(i).copied().unwrap_or(0);
            let before = table.get(i).copied().flatten();
            let diff = flag_diffs.get(i).map(Vec::as_slice).unwrap_or_default();
            let mut flags = match before {
                None if self.rinex == 2 => {
                    let mut f = diff.to_vec();
                    f.resize(f.len().max(2 * n), b' ');
                    f
                }
                None => Vec::new(),
                Some(p) => {
                    let mut f = self.flags.get(p).cloned().unwrap_or_default();
                    f.truncate(2 * n);
                    f
                }
            };
            repair(&mut flags, diff);

            let mut line_start = self.out.len();
            if self.rinex != 2 {
                self.out.extend_from_slice(sat);
            }
            let sat_arcs = arcs.get_mut(i).map(Vec::as_mut_slice).unwrap_or_default();
            for (j, arc) in sat_arcs.iter_mut().enumerate().take(n) {
                if arc.arc_order >= 0 {
                    let previous = before.and_then(|p| self.arcs.get(p)?.get(j)).map(|a| a.y);
                    restore(arc, previous).ok_or_else(|| self.err("an observation overflows"))?;
                    let value = get(&arc.y, usize::try_from(arc.order).unwrap_or(0));
                    let text = fixed(value, 3, 14)
                        .ok_or_else(|| self.err("an observation is too large for RINEX"))?;
                    self.out.extend_from_slice(text.as_bytes());
                    self.out.push(flag(&flags, 2 * j));
                    self.out.push(flag(&flags, 2 * j + 1));
                } else if self.crinex == 1 {
                    // CRINEX 1 takes a blank field's flags as blank.
                    self.out.extend_from_slice(&[b' '; 16]);
                    if flags.len() < 2 * j + 2 {
                        flags.resize(2 * j + 2, b' ');
                    }
                    for k in [2 * j, 2 * j + 1] {
                        if let Some(f) = flags.get_mut(k) {
                            *f = b' ';
                        }
                    }
                } else {
                    self.out.extend_from_slice(&[b' '; 14]);
                    self.out.push(flag(&flags, 2 * j));
                    self.out.push(flag(&flags, 2 * j + 1));
                }
                if j + 1 == n || (self.rinex == 2 && (j + 1) % 5 == 0) {
                    let len = line_start + trim_end(cols(&self.out, line_start, usize::MAX)).len();
                    self.out.truncate(len);
                    self.out.push(b'\n');
                    line_start = self.out.len();
                }
            }
            new_flags.push(flags);
        }
        self.flags = new_flags;
        Ok(())
    }
}

impl<R: BufRead> LineSource for Crinex<R> {
    fn read(&mut self, buf: &mut Vec<u8>) -> Result<bool, RinexError> {
        while self.at >= self.out.len() {
            if !self.fill()? {
                return Ok(false);
            }
        }
        let rest = cols(&self.out, self.at, usize::MAX);
        let end = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
        buf.clear();
        buf.extend_from_slice(cols(rest, 0, end));
        self.at += end + 1;
        self.given += 1;
        Ok(true)
    }

    fn line(&self) -> u64 {
        self.given
    }
}

/// Applies a text difference: a space keeps the old character, `&` makes
/// it a space, and anything else replaces it. A longer difference extends
/// the text.
fn repair(text: &mut Vec<u8>, diff: &[u8]) {
    for (i, &d) in diff.iter().enumerate() {
        let new = if d == b'&' { b' ' } else { d };
        match text.get_mut(i) {
            Some(old) if d != b' ' => *old = new,
            Some(_) => {}
            None => text.push(new),
        }
    }
}

/// Restores an arc's value from its differences and the previous epoch's.
/// `None` on overflow.
fn restore(arc: &mut Arc, previous: Option<[i64; MAX_ORDER + 1]>) -> Option<()> {
    let catching_up = arc.order < arc.arc_order;
    if catching_up {
        arc.order += 1;
    }
    let order = usize::try_from(arc.order).ok()?;
    for k in 0..order {
        let before = previous?;
        let b = if catching_up {
            get(&before, k)
        } else {
            get(&before, k + 1)
        };
        let sum = get(&arc.y, k).checked_add(b)?;
        set(&mut arc.y, k + 1, sum);
    }
    Some(())
}

fn get(values: &[i64; MAX_ORDER + 1], i: usize) -> i64 {
    values.get(i).copied().unwrap_or(0)
}

fn set(values: &mut [i64; MAX_ORDER + 1], i: usize, value: i64) {
    if let Some(slot) = values.get_mut(i) {
        *slot = value;
    }
}

/// The digit before `&` that starts an arc: its difference order, 0 to 5.
fn order_digit(field: &[u8]) -> Option<i32> {
    match field.first() {
        Some(&d @ b'0'..=b'5') => Some(i32::from(d - b'0')),
        _ => None,
    }
}

/// A signed integer of digits only.
fn integer(text: &[u8]) -> Option<i64> {
    let digits = text.strip_prefix(b"-").unwrap_or(text);
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(text).ok()?.parse().ok()
}

/// A flag character, blank if there isn't one.
fn flag(flags: &[u8], i: usize) -> u8 {
    flags.get(i).copied().unwrap_or(b' ')
}

/// A count of units of 10^-decimals as RINEX writes it, right-aligned in
/// `width`, the way CRX2RNX does: no zero before the point (`.123`,
/// `-.123`). `None` if it doesn't fit.
fn fixed(value: i64, decimals: u32, width: usize) -> Option<String> {
    let scale = 10_u64.checked_pow(decimals)?;
    let magnitude = value.unsigned_abs();
    let (whole, fraction) = (magnitude / scale, magnitude % scale);
    let sign = if value < 0 { "-" } else { "" };
    let digits = decimals as usize;
    let text = if whole == 0 {
        format!("{sign}.{fraction:0digits$}")
    } else {
        format!("{sign}{whole}.{fraction:0digits$}")
    };
    (text.len() <= width).then(|| format!("{text:>width$}"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::{fixed, integer, repair};

    #[test]
    fn text_differences_apply() {
        let mut text = b"> 2025 06 15 00 00".to_vec();
        repair(&mut text, b"                 3&ab");
        assert_eq!(text, b"> 2025 06 15 00 03 ab");
        let mut empty = Vec::new();
        repair(&mut empty, b"&1 2");
        assert_eq!(empty, b" 1 2");
    }

    #[test]
    fn values_print_as_crx2rnx_prints_them() {
        assert_eq!(fixed(123, 3, 14).unwrap(), "          .123");
        assert_eq!(fixed(-123, 3, 14).unwrap(), "         -.123");
        assert_eq!(fixed(0, 3, 14).unwrap(), "          .000");
        assert_eq!(fixed(1234, 3, 14).unwrap(), "         1.234");
        assert_eq!(fixed(-21_000_000_123, 3, 14).unwrap(), " -21000000.123");
        assert_eq!(fixed(9_999_999_999_999, 3, 14).unwrap(), "9999999999.999");
        assert_eq!(fixed(10_000_000_000_000, 3, 14), None);
        assert_eq!(fixed(-999_999_999_999, 3, 14).unwrap(), "-999999999.999");
        assert_eq!(fixed(-1_000_000_000_000, 3, 14), None);
        // Clock offsets: F12.9 in RINEX 2, F15.12 in RINEX 3.
        assert_eq!(fixed(-123_456_789, 9, 12).unwrap(), " -.123456789");
        assert_eq!(fixed(123, 12, 15).unwrap(), "  .000000000123");
        assert_eq!(fixed(12_000_000_000, 9, 12).unwrap(), "12.000000000");
        assert_eq!(fixed(-12_000_000_000, 9, 12), None);
    }

    #[test]
    fn differences_are_strict_integers() {
        assert_eq!(integer(b"-123"), Some(-123));
        assert_eq!(integer(b"0"), Some(0));
        assert_eq!(integer(b"-"), None);
        assert_eq!(integer(b"1 2"), None);
        assert_eq!(integer(b"+1"), None);
        assert_eq!(integer(b"99999999999999999999"), None);
    }
}
