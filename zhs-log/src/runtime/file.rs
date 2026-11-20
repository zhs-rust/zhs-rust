use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{LogError, LogFormatter, LogRecord, LogSink, Rotation, TextFormatter};

pub const DEFAULT_MAX_FILES: usize = 5;

pub struct FileSink {
    path: PathBuf,
    formatter: Box<dyn LogFormatter>,
    state: Mutex<FileState>,
    flush: bool,
    max_bytes: Option<u64>,
    max_files: usize,
    rotation: Rotation,
    utc_offset_seconds: i32,
}

struct FileState {
    writer: BufWriter<File>,
    written: u64,

    period: Option<u64>,
}

impl FileSink {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, LogError> {
        Self::with_formatter(path, TextFormatter::new())
    }

    pub fn with_formatter(
        path: impl Into<PathBuf>,
        formatter: impl LogFormatter + 'static,
    ) -> Result<Self, LogError> {
        let path = path.into();
        let (writer, written) = open_append(&path)?;

        Ok(Self {
            path,
            formatter: Box::new(formatter),
            state: Mutex::new(FileState {
                writer,
                written,
                period: None,
            }),
            flush: true,
            max_bytes: None,
            max_files: DEFAULT_MAX_FILES,
            rotation: Rotation::Never,
            utc_offset_seconds: 0,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn flush_on_write(&self) -> bool {
        self.flush
    }

    pub fn with_flush(mut self, flush: bool) -> Self {
        self.flush = flush;
        self
    }

    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = (max_bytes > 0).then_some(max_bytes);
        self
    }

    pub fn with_max_files(mut self, max_files: usize) -> Self {
        self.max_files = max_files.clamp(1, 999);
        self
    }

    pub fn with_rotation(mut self, rotation: Rotation) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_rotation_offset(mut self, utc_offset_seconds: i32) -> Self {
        self.utc_offset_seconds = utc_offset_seconds;
        self
    }

    pub fn max_bytes(&self) -> Option<u64> {
        self.max_bytes
    }

    pub fn max_files(&self) -> usize {
        self.max_files
    }

    pub fn rotation(&self) -> Rotation {
        self.rotation
    }

    pub fn rotation_offset(&self) -> i32 {
        self.utc_offset_seconds
    }

    fn rotate_by_size(&self, state: &mut FileState) -> Result<(), LogError> {
        state
            .writer
            .flush()
            .map_err(|error| LogError::write_file(&self.path, error))?;

        let oldest = rotated_path(&self.path, self.max_files);
        match fs::remove_file(&oldest) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(LogError::write_file(&oldest, error)),
        }

        for index in (1..self.max_files).rev() {
            let from = rotated_path(&self.path, index);
            let to = rotated_path(&self.path, index + 1);

            match fs::rename(&from, &to) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(LogError::write_file(&from, error)),
            }
        }

        fs::rename(&self.path, rotated_path(&self.path, 1))
            .map_err(|error| LogError::write_file(&self.path, error))?;

        let (writer, written) = open_append(&self.path)?;
        state.writer = writer;
        state.written = written;
        Ok(())
    }

    fn rotate_by_time(&self, state: &mut FileState, period: u64) -> Result<(), LogError> {
        state
            .writer
            .flush()
            .map_err(|error| LogError::write_file(&self.path, error))?;

        let suffix = self.rotation.suffix(period, self.utc_offset_seconds);
        let archive = unique_archive_path(&self.path, &suffix);

        fs::rename(&self.path, &archive)
            .map_err(|error| LogError::write_file(&self.path, error))?;

        let (writer, written) = open_append(&self.path)?;
        state.writer = writer;
        state.written = written;
        Ok(())
    }

    fn content_period(&self) -> Option<u64> {
        if !self.rotation.is_enabled() {
            return None;
        }

        let modified = self
            .path
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_else(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_secs())
                    .unwrap_or(0)
            });

        self.rotation
            .period_start(modified, self.utc_offset_seconds)
    }
}

fn open_append(path: &Path) -> Result<(BufWriter<File>, u64), LogError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| LogError::open(path, error))?;
    }

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| LogError::open(path, error))?;
    let written = file
        .metadata()
        .map_err(|error| LogError::open(path, error))?
        .len();

    Ok((BufWriter::new(file), written))
}

fn rotated_path(path: &Path, index: usize) -> PathBuf {
    suffixed_path(path, &index.to_string())
}

fn suffixed_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".");
    name.push(suffix);
    PathBuf::from(name)
}

fn unique_archive_path(path: &Path, suffix: &str) -> PathBuf {
    let mut candidate = suffixed_path(path, suffix);
    let mut counter = 1;

    // Search until a free name is found; a bounded search would hand back an
    // existing path and the subsequent rename would clobber that archive.
    while candidate.exists() {
        candidate = suffixed_path(path, &format!("{suffix}-{counter}"));
        counter += 1;
    }

    candidate
}

impl LogSink for FileSink {
    fn write(&self, record: &LogRecord) -> Result<(), LogError> {
        let line = self.formatter.format(record);
        let bytes = line.as_bytes();
        let mut state = lock(&self.state);

        if let Some(period) = self
            .rotation
            .period_start(record.timestamp().unix_seconds(), self.utc_offset_seconds)
        {
            let current = state.period.or_else(|| self.content_period());

            match current {
                Some(current) if current != period => {
                    if state.written > 0 {
                        self.rotate_by_time(&mut state, current)?;
                    }
                    state.period = Some(period);
                }
                Some(current) => state.period = Some(current),
                None => state.period = Some(period),
            }
        }

        if let Some(max_bytes) = self.max_bytes {
            let additional = bytes.len() as u64 + 1;

            if state.written > 0 && state.written.saturating_add(additional) > max_bytes {
                self.rotate_by_size(&mut state)?;
            }
        }

        state
            .writer
            .write_all(bytes)
            .and_then(|()| state.writer.write_all(b"\n"))
            .map_err(|error| LogError::write_file(&self.path, error))?;
        state.written = state.written.saturating_add(bytes.len() as u64 + 1);

        if self.flush {
            state
                .writer
                .flush()
                .map_err(|error| LogError::write_file(&self.path, error))?;
        }

        Ok(())
    }

    fn flush(&self) -> Result<(), LogError> {
        lock(&self.state)
            .writer
            .flush()
            .map_err(|error| LogError::write_file(&self.path, error))
    }
}

impl Drop for FileSink {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

impl fmt::Debug for FileSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileSink")
            .field("path", &self.path)
            .field("flush", &self.flush)
            .field("max_bytes", &self.max_bytes)
            .field("max_files", &self.max_files)
            .field("rotation", &self.rotation)
            .field("utc_offset_seconds", &self.utc_offset_seconds)
            .finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::FileSink;
    use crate::runtime::{LogLevel, LogRecord, LogSink, Rotation, Timestamp};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zhs-log-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn sink(path: &Path) -> FileSink {
        FileSink::with_formatter(path, |record: &LogRecord| record.message().to_owned())
            .expect("open the log file")
    }

    fn record(unix_seconds: u64, message: &'static str) -> LogRecord {
        LogRecord::at(
            Timestamp::from_system_time(UNIX_EPOCH + Duration::from_secs(unix_seconds)),
            LogLevel::Info,
            "test",
            message,
        )
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap_or_default()
    }

    const MIDNIGHT_UTC: u64 = 1_794_585_600;

    #[test]
    fn rotates_when_the_period_changes() {
        let dir = temp_dir("daily");
        let path = dir.join("app.log");

        let sink = sink(&path)
            .with_rotation(Rotation::Daily)
            .with_rotation_offset(28_800);

        sink.write(&record(MIDNIGHT_UTC - 60, "before midnight"))
            .unwrap();
        assert_eq!(read(&path), "before midnight\n");

        sink.write(&record(MIDNIGHT_UTC + 60, "after midnight"))
            .unwrap();
        assert_eq!(read(&dir.join("app.log.2026-11-13")), "before midnight\n");
        assert_eq!(read(&path), "after midnight\n");

        sink.write(&record(MIDNIGHT_UTC + 86_400 + 60, "next day"))
            .unwrap();
        assert_eq!(read(&dir.join("app.log.2026-11-14")), "after midnight\n");
        assert_eq!(read(&path), "next day\n");
    }

    #[test]
    fn archives_a_stale_file_on_the_first_write_after_a_restart() {
        let dir = temp_dir("restart");
        let path = dir.join("app.log");

        {
            let sink = sink(&path)
                .with_rotation(Rotation::Daily)
                .with_rotation_offset(28_800);
            sink.write(&record(MIDNIGHT_UTC - 60, "yesterday")).unwrap();
        }

        let file = fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open the log file");
        file.set_times(
            fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(MIDNIGHT_UTC - 60)),
        )
        .expect("set the modification time");

        let sink = sink(&path)
            .with_rotation(Rotation::Daily)
            .with_rotation_offset(28_800);
        sink.write(&record(MIDNIGHT_UTC + 60, "today")).unwrap();

        assert_eq!(read(&dir.join("app.log.2026-11-13")), "yesterday\n");
        assert_eq!(read(&path), "today\n");
    }

    #[test]
    fn does_not_rotate_when_disabled() {
        let dir = temp_dir("off");
        let path = dir.join("app.log");

        let sink = sink(&path);
        sink.write(&record(MIDNIGHT_UTC - 60, "one")).unwrap();
        sink.write(&record(MIDNIGHT_UTC + 86_400, "two")).unwrap();

        assert_eq!(read(&path), "one\ntwo\n");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn names_interval_archives_with_the_period_start() {
        let dir = temp_dir("interval");
        let path = dir.join("app.log");

        let sink = sink(&path).with_rotation(Rotation::parse("30m").unwrap());

        sink.write(&record(MIDNIGHT_UTC, "first")).unwrap();
        sink.write(&record(MIDNIGHT_UTC + 1_800, "second")).unwrap();
        sink.write(&record(MIDNIGHT_UTC + 1_860, "third")).unwrap();
        sink.write(&record(MIDNIGHT_UTC + 3_600, "fourth")).unwrap();

        assert_eq!(read(&dir.join("app.log.2026-11-13-1600")), "first\n");
        assert_eq!(
            read(&dir.join("app.log.2026-11-13-1630")),
            "second\nthird\n"
        );
        assert_eq!(read(&path), "fourth\n");
    }

    #[test]
    fn keeps_size_rotation_working_alongside_time_rotation() {
        let dir = temp_dir("mixed");
        let path = dir.join("app.log");

        let sink = sink(&path)
            .with_rotation(Rotation::Daily)
            .with_rotation_offset(0)
            .with_max_bytes(10);

        sink.write(&record(MIDNIGHT_UTC, "0123456789")).unwrap();
        sink.write(&record(MIDNIGHT_UTC + 1, "abcdefghij")).unwrap();

        assert!(
            dir.join("app.log.1").exists(),
            "size rotation still applies"
        );
        assert_eq!(
            read(&dir.join("app.log.1")),
            "0123456789\n",
            "the size archive holds the first line"
        );
    }

    #[test]
    fn keeps_the_timestamp_helper_in_sync_with_the_fixture() {
        let timestamp =
            Timestamp::from_system_time(SystemTime::UNIX_EPOCH + Duration::from_secs(MIDNIGHT_UTC));
        assert_eq!(timestamp.format(28_800), "2026-11-14 00:00:00.000+08:00");
    }
}
