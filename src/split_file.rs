use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

//use crate::utils::{format_duration, Prefix, Sign};

#[derive(Serialize, Deserialize, Debug)]
pub struct SplitFile {
    pub title: String,
    pub category: String,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub completed: u32,
    pub split_names: Vec<String>,
    #[serde(default)]
    pub golds: Vec<Option<Segment>>,
    #[serde(default)]
    pub shits: Vec<Option<Segment>>,
    #[serde(default)]
    pub averages: Vec<Option<Segment>>,
    #[serde(default)]
    pub personal_best: Option<ActualRun>,
    #[serde(default)]
    pub personal_worst: Option<ActualRun>,
    #[serde(default)]
    pub total_runtime: Duration,

    // Clears counter
    #[serde(default)]
    pub clears: Vec<u32>,

    #[serde(skip)]
    file_path: PathBuf,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Segment {
    #[serde(with = "duration_format")]
    pub duration: Duration,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ActualRun {
    pub attempt: u32,
    pub splits: Vec<Option<Split>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Split {
    #[serde(with = "duration_format")]
    pub time: Duration,
}

mod duration_format {
    use core::fmt;
    use std::str::FromStr;
    use std::{sync::LazyLock, time::Duration};

    use regex::{Match, Regex};
    use serde::{de, Deserializer, Serializer};

    use crate::utils::{format_duration, Prefix, Sign};

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format_duration(
            *duration,
            3,
            Sign::Positive,
            Prefix::NoneOrMinus,
        ))
    }

    static DURATION_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:(?:(?:(\d+):)?(\d+):)?(\d+):)?(\d+)(?:\.(\d{1,3}))?$").unwrap()
    });

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DurationVisitor;

        impl<'de> de::Visitor<'de> for DurationVisitor {
            type Value = Duration;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string in the format D:H:MM:SS.mmm")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let caps = DURATION_REGEX
                    .captures(value)
                    .ok_or_else(|| E::custom("invalid duration format"))?;

                let days = get_or_zero(&caps.get(1))?;
                let hours = get_or_zero(&caps.get(2))?;
                let minutes = get_or_zero(&caps.get(3))?;
                let seconds = get_or_zero(&caps.get(4))?;
                let milliseconds = get_or_zero_padded(&caps.get(5))?;

                let secs = seconds + (minutes * 60) + (hours * 60 * 60) + (days * 60 * 60 * 24);
                let dur = Duration::from_secs(secs) + Duration::from_millis(milliseconds);
                Ok(dur)
            }
        }

        deserializer.deserialize_str(DurationVisitor)
    }

    fn get_or_zero<E>(mat: &Option<Match<'_>>) -> Result<u64, E>
    where
        E: de::Error,
    {
        let num = match mat {
            Some(d) => u64::from_str(d.as_str()).map_err(E::custom)?,
            None => 0,
        };
        Ok(num)
    }

    fn get_or_zero_padded<E>(mat: &Option<Match<'_>>) -> Result<u64, E>
    where
        E: de::Error,
    {
        let num = match mat {
            Some(d) => {
                let mut num_str = d.as_str().to_string();
                while num_str.len() < 3 {
                    num_str.push('0');
                }
                u64::from_str(&num_str).unwrap()
            }
            None => 0,
        };
        Ok(num)
    }
}

pub fn read_split_file(path: &Path) -> anyhow::Result<SplitFile> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let mut split_file: SplitFile = serde_json::from_reader(reader)?;
    split_file.file_path = path.to_owned();

    if split_file.split_names.is_empty() {
        return Err(anyhow!("Split names cannot be empty"));
    }
    
    if Some(split_file.total_runtime).is_none() {
        split_file.total_runtime = Duration::ZERO;
    }

    if split_file.golds.is_empty() {
        split_file.golds = vec![None; split_file.split_names.len()];
    }
    
    if split_file.shits.is_empty() {
        split_file.shits = vec![None; split_file.split_names.len()];
    }
    
    if split_file.averages.is_empty() {
        split_file.averages = vec![Some(Segment{duration: Duration::ZERO}); split_file.split_names.len()];
    }

    if split_file.clears.is_empty() {
        split_file.clears = vec![0; split_file.split_names.len()];
    }

    Ok(split_file)
}

pub fn write_split_file(split_file: &SplitFile) -> anyhow::Result<()> {
    let file = std::fs::File::create(&split_file.file_path)?;
    serde_json::to_writer_pretty(file, split_file)?;
    Ok(())
}
