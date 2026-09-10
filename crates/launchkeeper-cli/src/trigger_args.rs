//! Command-line flags → [`Trigger`], kept pure so it is trivial to unit test.

use clap::Args;
use launchkeeper_core::{CalendarEntry, Trigger};

/// The trigger-selecting flags shared by `add` and `set-trigger`. Exactly one
/// of these must be given (`clap` enforces that with a required, non-multi
/// `ArgGroup`).
#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
pub struct TriggerArgs {
    /// `RunAtLoad`: fire once when the agent is loaded (i.e. at login).
    #[arg(long = "at-login")]
    pub at_login: bool,

    /// `StartInterval`, e.g. `30m`, `2h`, `90s`, `1h30m`.
    #[arg(long, short = 'e')]
    pub every: Option<String>,

    /// A daily time, `HH:MM`. Repeatable for several times a day.
    #[arg(long, short = 'd')]
    pub daily: Vec<String>,

    /// `weekday[,weekday...]@HH:MM`, e.g. `mon,fri@09:30`. Repeatable.
    #[arg(long, short = 'w')]
    pub weekly: Vec<String>,

    /// `day@HH:MM`, e.g. `15@08:00`. Repeatable.
    #[arg(long, short = 'M')]
    pub monthly: Vec<String>,

    /// No automatic trigger: a long-running service started and stopped by
    /// hand (`start` / `stop` / `restart`). Pair with `--keep-alive` to have
    /// launchd restart it after a crash.
    #[arg(long, short = 'm')]
    pub manual: bool,
}

/// Turns the parsed flags into a [`Trigger`]. Callers still need to call
/// [`Trigger::validate`] (this only handles syntax, not range checks beyond
/// what parsing itself requires).
///
/// # Errors
/// A human-readable message describing what was wrong.
pub fn trigger_from_args(args: &TriggerArgs) -> Result<Trigger, String> {
    if args.at_login {
        return Ok(Trigger::AtLogin);
    }
    if args.manual {
        return Ok(Trigger::Manual);
    }
    if let Some(every) = &args.every {
        let seconds = parse_duration(every)?;
        return Ok(Trigger::Interval { seconds });
    }
    if !args.daily.is_empty() {
        let mut entries = Vec::new();
        for d in &args.daily {
            let (hour, minute) = parse_hhmm(d)?;
            entries.push(CalendarEntry::daily(hour, minute));
        }
        return Ok(Trigger::Calendar { entries });
    }
    if !args.weekly.is_empty() {
        let mut entries = Vec::new();
        for w in &args.weekly {
            entries.extend(parse_weekly(w)?);
        }
        return Ok(Trigger::Calendar { entries });
    }
    if !args.monthly.is_empty() {
        let mut entries = Vec::new();
        for m in &args.monthly {
            entries.push(parse_monthly(m)?);
        }
        return Ok(Trigger::Calendar { entries });
    }
    // The ArgGroup requires one of the above, so this is unreachable when
    // flags are parsed through clap; kept for direct callers/tests.
    Err(
        "必须指定一种触发方式：--at-login / --every / --daily / --weekly / --monthly / --manual"
            .to_string(),
    )
}

/// Parses `30m`, `2h`, `90s`, `1h30m`, `1h30m10s`, ... into seconds.
///
/// Also used by `--timeout`, hence the error messages naming `--every` are
/// generic enough to read either way.
///
/// # Errors
/// A human-readable message; overflowing values are rejected rather than
/// wrapping.
pub fn parse_duration(s: &str) -> Result<u32, String> {
    if s.is_empty() {
        return Err("--every 不能为空".to_string());
    }
    let mut total: u64 = 0;
    let mut chars = s.chars().peekable();
    let mut saw_component = false;
    while chars.peek().is_some() {
        let mut digits = String::new();
        while let Some(c) = chars.peek() {
            if c.is_ascii_digit() {
                digits.push(*c);
                chars.next();
            } else {
                break;
            }
        }
        if digits.is_empty() {
            return Err(format!(
                "--every 格式错误: {s:?}，期望形如 30m/2h/90s/1h30m"
            ));
        }
        let Some(unit) = chars.next() else {
            return Err(format!("--every 缺少单位: {s:?}，期望 h/m/s"));
        };
        let n: u64 = digits.parse().map_err(|_| format!("时长数字过大: {s:?}"))?;
        let mul: u64 = match unit {
            'h' => 3600,
            'm' => 60,
            's' => 1,
            other => return Err(format!("--every 未知单位 {other:?}: {s:?}，期望 h/m/s")),
        };
        total = n
            .checked_mul(mul)
            .and_then(|v| total.checked_add(v))
            .ok_or_else(|| format!("时长溢出: {s:?}"))?;
        saw_component = true;
    }
    if !saw_component || total == 0 {
        return Err(format!("--every 格式错误: {s:?}"));
    }
    u32::try_from(total).map_err(|_| format!("--every 太大了: {s:?}"))
}

/// Parses `HH:MM` into `(hour, minute)`, validating ranges.
fn parse_hhmm(s: &str) -> Result<(u8, u8), String> {
    let (h, m) = s
        .split_once(':')
        .ok_or_else(|| format!("时间格式错误: {s:?}，期望 HH:MM"))?;
    let hour: u8 = h.parse().map_err(|_| format!("小时不是数字: {s:?}"))?;
    let minute: u8 = m.parse().map_err(|_| format!("分钟不是数字: {s:?}"))?;
    if hour > 23 {
        return Err(format!("小时必须在 0-23: {s:?}"));
    }
    if minute > 59 {
        return Err(format!("分钟必须在 0-59: {s:?}"));
    }
    Ok((hour, minute))
}

/// Parses one weekday token: `mon`..`sun` (English three-letter abbreviations)
/// or a launchd-style number `0`-`7`.
fn parse_weekday(s: &str) -> Result<u8, String> {
    let lower = s.to_ascii_lowercase();
    let w = match lower.as_str() {
        "sun" => 0,
        "mon" => 1,
        "tue" => 2,
        "wed" => 3,
        "thu" => 4,
        "fri" => 5,
        "sat" => 6,
        other => {
            return other
                .parse::<u8>()
                .ok()
                .filter(|w| *w <= 7)
                .ok_or_else(|| format!("星期格式错误: {s:?}，期望 mon..sun 或 0-7"));
        }
    };
    Ok(w)
}

/// Parses `weekday[,weekday...]@HH:MM` into one entry per weekday.
fn parse_weekly(s: &str) -> Result<Vec<CalendarEntry>, String> {
    let (days, time) = s
        .split_once('@')
        .ok_or_else(|| format!("--weekly 格式错误: {s:?}，期望 mon,fri@HH:MM"))?;
    if days.is_empty() {
        return Err(format!("--weekly 缺少星期: {s:?}"));
    }
    let (hour, minute) = parse_hhmm(time)?;
    let mut entries = Vec::new();
    for d in days.split(',') {
        let d = d.trim();
        if d.is_empty() {
            return Err(format!("--weekly 星期列表里有空项: {s:?}"));
        }
        let weekday = parse_weekday(d)?;
        entries.push(CalendarEntry {
            minute,
            hour,
            weekday: Some(weekday),
            day: None,
        });
    }
    Ok(entries)
}

/// Parses `day@HH:MM`.
fn parse_monthly(s: &str) -> Result<CalendarEntry, String> {
    let (day, time) = s
        .split_once('@')
        .ok_or_else(|| format!("--monthly 格式错误: {s:?}，期望 15@HH:MM"))?;
    let day: u8 = day
        .parse()
        .map_err(|_| format!("--monthly 日期不是数字: {s:?}"))?;
    if !(1..=31).contains(&day) {
        return Err(format!("--monthly 日期必须在 1-31: {s:?}"));
    }
    let (hour, minute) = parse_hhmm(time)?;
    Ok(CalendarEntry {
        minute,
        hour,
        weekday: None,
        day: Some(day),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> TriggerArgs {
        TriggerArgs {
            at_login: false,
            every: None,
            daily: Vec::new(),
            weekly: Vec::new(),
            monthly: Vec::new(),
            manual: false,
        }
    }

    #[test]
    fn at_login() {
        let a = TriggerArgs {
            at_login: true,
            ..args()
        };
        assert_eq!(trigger_from_args(&a).unwrap(), Trigger::AtLogin);
    }

    #[test]
    fn manual() {
        let a = TriggerArgs {
            manual: true,
            ..args()
        };
        assert_eq!(trigger_from_args(&a).unwrap(), Trigger::Manual);
    }

    #[test]
    fn every_forms() {
        for (input, secs) in [
            ("30m", 1800),
            ("2h", 7200),
            ("90s", 90),
            ("1h30m", 5400),
            ("1h30m10s", 5410),
        ] {
            let a = TriggerArgs {
                every: Some(input.to_string()),
                ..args()
            };
            assert_eq!(
                trigger_from_args(&a).unwrap(),
                Trigger::Interval { seconds: secs },
                "input {input}"
            );
        }
    }

    #[test]
    fn every_errors() {
        for bad in ["", "30", "m", "30x", "h30"] {
            let a = TriggerArgs {
                every: Some(bad.to_string()),
                ..args()
            };
            assert!(trigger_from_args(&a).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn daily_single_and_repeated() {
        let a = TriggerArgs {
            daily: vec!["21:00".into()],
            ..args()
        };
        assert_eq!(
            trigger_from_args(&a).unwrap(),
            Trigger::Calendar {
                entries: vec![CalendarEntry::daily(21, 0)]
            }
        );

        let a = TriggerArgs {
            daily: vec!["09:00".into(), "21:30".into()],
            ..args()
        };
        assert_eq!(
            trigger_from_args(&a).unwrap(),
            Trigger::Calendar {
                entries: vec![CalendarEntry::daily(9, 0), CalendarEntry::daily(21, 30)]
            }
        );
    }

    #[test]
    fn daily_bad_time() {
        for bad in ["25:00", "12:60", "noon", "12", "12:"] {
            let a = TriggerArgs {
                daily: vec![bad.into()],
                ..args()
            };
            assert!(trigger_from_args(&a).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn weekly_names_and_numbers() {
        let a = TriggerArgs {
            weekly: vec!["mon,fri@09:30".into()],
            ..args()
        };
        assert_eq!(
            trigger_from_args(&a).unwrap(),
            Trigger::Calendar {
                entries: vec![
                    CalendarEntry {
                        minute: 30,
                        hour: 9,
                        weekday: Some(1),
                        day: None
                    },
                    CalendarEntry {
                        minute: 30,
                        hour: 9,
                        weekday: Some(5),
                        day: None
                    },
                ]
            }
        );

        let a = TriggerArgs {
            weekly: vec!["0,7@08:00".into()],
            ..args()
        };
        assert_eq!(
            trigger_from_args(&a).unwrap(),
            Trigger::Calendar {
                entries: vec![
                    CalendarEntry {
                        minute: 0,
                        hour: 8,
                        weekday: Some(0),
                        day: None
                    },
                    CalendarEntry {
                        minute: 0,
                        hour: 8,
                        weekday: Some(7),
                        day: None
                    },
                ]
            }
        );
    }

    #[test]
    fn weekly_repeated_flag_accumulates() {
        let a = TriggerArgs {
            weekly: vec!["mon@09:00".into(), "fri@18:00".into()],
            ..args()
        };
        let t = trigger_from_args(&a).unwrap();
        match t {
            Trigger::Calendar { entries } => assert_eq!(entries.len(), 2),
            _ => panic!("expected calendar"),
        }
    }

    #[test]
    fn weekly_bad_weekday_and_empty() {
        for bad in ["xyz@09:00", "@09:00", "mon@25:00", "mon,@09:00"] {
            let a = TriggerArgs {
                weekly: vec![bad.into()],
                ..args()
            };
            assert!(trigger_from_args(&a).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn monthly_valid_and_invalid() {
        let a = TriggerArgs {
            monthly: vec!["15@08:00".into()],
            ..args()
        };
        assert_eq!(
            trigger_from_args(&a).unwrap(),
            Trigger::Calendar {
                entries: vec![CalendarEntry {
                    minute: 0,
                    hour: 8,
                    weekday: None,
                    day: Some(15)
                }]
            }
        );

        for bad in ["0@08:00", "32@08:00", "abc@08:00", "15@25:00", "15"] {
            let a = TriggerArgs {
                monthly: vec![bad.into()],
                ..args()
            };
            assert!(trigger_from_args(&a).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn nothing_set_is_an_error() {
        assert!(trigger_from_args(&args()).is_err());
    }
}
