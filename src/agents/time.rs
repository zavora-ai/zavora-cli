/// Native time agent for deterministic time/date operations.
use chrono::{DateTime, Datelike, Duration, Utc, Weekday};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeContext {
    pub now_iso: String,
    pub timezone: String,
    pub weekday: String,
    pub date: String,
}

pub struct TimeAgent;

impl TimeAgent {
    /// Generate time handshake for session start.
    pub fn handshake() -> TimeContext {
        let now = Utc::now();
        TimeContext {
            now_iso: now.to_rfc3339(),
            timezone: "UTC".to_string(),
            weekday: now.weekday().to_string(),
            date: now.date_naive().to_string(),
        }
    }

    /// Parse relative time expressions like "next Friday", "in 2 days".
    pub fn parse_relative(input: &str) -> anyhow::Result<DateTime<Utc>> {
        let now = Utc::now();
        let input = input.trim().to_lowercase();

        // Simple patterns
        if input == "now" || input == "today" {
            return Ok(now);
        }

        if input == "tomorrow" {
            return Ok(now + Duration::days(1));
        }

        if input == "yesterday" {
            return Ok(now - Duration::days(1));
        }

        // "in X days/hours/minutes"
        if let Some(rest) = input.strip_prefix("in ") {
            if let Some((num_str, unit)) = rest.split_once(' ') {
                if let Ok(num) = num_str.parse::<i64>() {
                    let delta = match unit {
                        "minute" | "minutes" => Duration::minutes(num),
                        "hour" | "hours" => Duration::hours(num),
                        "day" | "days" => Duration::days(num),
                        "week" | "weeks" => Duration::weeks(num),
                        _ => return Err(anyhow::anyhow!("Unknown time unit: {}", unit)),
                    };
                    return Ok(now + delta);
                }
            }
        }

        // "next Friday"
        if let Some(weekday_str) = input.strip_prefix("next ") {
            if let Some(target_weekday) = parse_weekday(weekday_str) {
                return Ok(next_weekday(now, target_weekday));
            }
        }

        Err(anyhow::anyhow!("Could not parse relative time: {}", input))
    }

    /// Perform time arithmetic: base + delta.
    pub fn time_arithmetic(base: DateTime<Utc>, delta: &str) -> anyhow::Result<DateTime<Utc>> {
        let delta = delta.trim();
        
        if let Some(rest) = delta.strip_prefix('+') {
            let duration = parse_duration(rest)?;
            Ok(base + duration)
        } else if let Some(rest) = delta.strip_prefix('-') {
            let duration = parse_duration(rest)?;
            Ok(base - duration)
        } else {
            Err(anyhow::anyhow!("Delta must start with + or -: {}", delta))
        }
    }
}

fn parse_weekday(s: &str) -> Option<Weekday> {
    match s.to_lowercase().as_str() {
        "monday" | "mon" => Some(Weekday::Mon),
        "tuesday" | "tue" => Some(Weekday::Tue),
        "wednesday" | "wed" => Some(Weekday::Wed),
        "thursday" | "thu" => Some(Weekday::Thu),
        "friday" | "fri" => Some(Weekday::Fri),
        "saturday" | "sat" => Some(Weekday::Sat),
        "sunday" | "sun" => Some(Weekday::Sun),
        _ => None,
    }
}

fn next_weekday(from: DateTime<Utc>, target: Weekday) -> DateTime<Utc> {
    let current = from.weekday();
    let days_ahead = ((target.num_days_from_monday() as i64
        - current.num_days_from_monday() as i64
        + 7)
        % 7) as i64;
    let days_ahead = if days_ahead == 0 { 7 } else { days_ahead };
    from + Duration::days(days_ahead)
}

fn parse_duration(s: &str) -> anyhow::Result<Duration> {
    let s = s.trim();
    if let Some((num_str, unit)) = s.split_once(' ') {
        if let Ok(num) = num_str.parse::<i64>() {
            return Ok(match unit {
                "minute" | "minutes" => Duration::minutes(num),
                "hour" | "hours" => Duration::hours(num),
                "day" | "days" => Duration::days(num),
                "week" | "weeks" => Duration::weeks(num),
                _ => return Err(anyhow::anyhow!("Unknown time unit: {}", unit)),
            });
        }
    }
    Err(anyhow::anyhow!("Could not parse duration: {}", s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handshake() {
        let ctx = TimeAgent::handshake();
        assert!(!ctx.now_iso.is_empty());
        assert_eq!(ctx.timezone, "UTC");
    }

    #[test]
    fn test_parse_relative() {
        let now = TimeAgent::parse_relative("now").unwrap();
        let tomorrow = TimeAgent::parse_relative("tomorrow").unwrap();
        assert!(tomorrow > now);
    }
}
