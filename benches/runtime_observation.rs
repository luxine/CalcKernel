//! Bounded, optional observations around a gate call; never an alternative timer or verdict.
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
};

const MAX_ROWS: usize = 3 * 3 + 20 * 7 * 3;
const FIELDS: [&str; 9] = [
    "wallNs",
    "threadCpuNs",
    "cpu",
    "userCpuNs",
    "systemCpuNs",
    "minorFaults",
    "majorFaults",
    "voluntarySwitches",
    "involuntarySwitches",
];

#[derive(Default)]
pub struct Snapshot {
    values: [Option<u128>; 9],
}

impl Snapshot {
    pub fn capture() -> Self {
        #[cfg(target_os = "linux")]
        {
            fn clock(id: libc::clockid_t) -> Option<u128> {
                let mut time = std::mem::MaybeUninit::<libc::timespec>::uninit();
                // SAFETY: the pointer has the size/alignment required by clock_gettime.
                if unsafe { libc::clock_gettime(id, time.as_mut_ptr()) } != 0 {
                    return None;
                }
                // SAFETY: a successful clock_gettime initialized the structure.
                let time = unsafe { time.assume_init() };
                let nanos = u128::try_from(time.tv_nsec).ok()?;
                (nanos < 1_000_000_000)
                    .then_some(u128::try_from(time.tv_sec).ok()? * 1_000_000_000 + nanos)
            }
            fn cpu_time(time: libc::timeval) -> Option<u128> {
                let micros = u128::try_from(time.tv_usec).ok()?;
                (micros < 1_000_000)
                    .then_some(u128::try_from(time.tv_sec).ok()? * 1_000_000_000 + micros * 1000)
            }
            let mut result = Self::default();
            result.values[0] = clock(libc::CLOCK_MONOTONIC_RAW);
            result.values[1] = clock(libc::CLOCK_THREAD_CPUTIME_ID);
            // SAFETY: sched_getcpu has no pointer arguments.
            result.values[2] = u128::try_from(unsafe { libc::sched_getcpu() }).ok();
            let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
            // SAFETY: the pointer is valid; failed reads remain unavailable, not zero.
            if unsafe { libc::getrusage(libc::RUSAGE_THREAD, usage.as_mut_ptr()) } == 0 {
                // SAFETY: successful getrusage initialized the structure.
                let usage = unsafe { usage.assume_init() };
                result.values[3] = cpu_time(usage.ru_utime);
                result.values[4] = cpu_time(usage.ru_stime);
                for (slot, value) in result.values[5..].iter_mut().zip([
                    usage.ru_minflt,
                    usage.ru_majflt,
                    usage.ru_nvcsw,
                    usage.ru_nivcsw,
                ]) {
                    *slot = u128::try_from(value).ok();
                }
            }
            result
        }
        #[cfg(not(target_os = "linux"))]
        Self::default()
    }

    pub fn json(&self) -> String {
        let fields: Vec<_> = FIELDS
            .iter()
            .zip(self.values)
            .map(|(name, value)| format!("\"{name}\":{}", number(value)))
            .collect();
        format!("{{{}}}", fields.join(","))
    }
}

fn number(value: Option<u128>) -> String {
    value.map_or_else(|| "null".into(), |value| value.to_string())
}

pub fn json_string(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            c if c < ' ' => output.push_str(&format!("\\u{:04x}", c as u32)),
            c => output.push(c),
        }
    }
    output.push('"');
    output
}

struct Row {
    channel: usize,
    warmup: bool,
    gate_ns: Option<u128>,
    before: Snapshot,
    after: Snapshot,
}

pub struct Collector {
    file: File,
    rows: Vec<Row>,
    overflow: bool,
}

impl Collector {
    pub fn new(path: &Path, identity: &str) -> io::Result<Self> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        writeln!(file, "{identity}")?;
        Ok(Self {
            file,
            rows: Vec::with_capacity(MAX_ROWS),
            overflow: false,
        })
    }

    pub fn measure<E>(
        &mut self,
        channel: usize,
        warmup: bool,
        call: impl FnOnce() -> Result<u128, E>,
    ) -> Result<u128, E> {
        let before = Snapshot::capture();
        let result = call();
        let after = Snapshot::capture();
        if self.rows.len() < MAX_ROWS {
            self.rows.push(Row {
                channel,
                warmup,
                gate_ns: result.as_ref().ok().copied(),
                before,
                after,
            });
        } else {
            self.overflow = true;
        }
        result
    }

    pub fn finish(mut self, sampling_succeeded: bool) -> io::Result<()> {
        for (sequence, row) in self.rows.iter().enumerate() {
            writeln!(
                self.file,
                "{{\"type\":\"sample\",\"sequence\":{sequence},\"channel\":{},\"warmup\":{},\"gateNs\":{},\"before\":{},\"after\":{}}}",
                row.channel,
                row.warmup,
                number(row.gate_ns),
                row.before.json(),
                row.after.json()
            )?;
        }
        writeln!(
            self.file,
            "{{\"type\":\"complete\",\"rows\":{},\"samplingSucceeded\":{sampling_succeeded},\"overflow\":{}}}",
            self.rows.len(),
            self.overflow
        )?;
        self.file.flush()
    }
}
