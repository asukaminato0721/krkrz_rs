//! TJS Date uses whole Unix seconds, local calendar components, and a distinct
//! date-string grammar. Based on upstream tjsDate.cpp and syntax/tjsdate.y.
use crate::{Host, Value, Vm};
use anyhow::{Context, Result, bail, ensure};
use jiff::{SignedDuration, Timestamp, Zoned, civil::DateTime, tz::TimeZone};

fn calendar(parts: [i32; 6]) -> Result<DateTime> {
    let [year, month, day, hour, minute, second] = parts.map(i64::from);
    let year = year + month.div_euclid(12);
    let month = month.rem_euclid(12) + 1;
    let date = DateTime::new(year.try_into()?, month as i8, 1, 0, 0, 0, 0)
        .context("invalid Date timestamp")?;
    date.checked_add(SignedDuration::from_secs(
        (day - 1) * 86400 + hour * 3600 + minute * 60 + second,
    ))
    .context("invalid Date timestamp")
}

// tjsDate's constructor supplies tm_isdst=0. Setters preserve the old DST
// adjustment when normalizing changed components, including seasonal crossings.
fn standard_offset(year: i16) -> Result<i32> {
    standard_offset_in(&TimeZone::system(), year)
}
fn standard_offset_in(zone: &TimeZone, year: i16) -> Result<i32> {
    let mut offsets = Vec::new();
    for month in [1, 7] {
        let date = DateTime::new(year, month, 15, 12, 0, 0, 0)?;
        offsets.push(zone.to_zoned(date)?.offset().seconds());
    }
    Ok(*offsets.iter().min().unwrap())
}
fn local(seconds: i64) -> Result<Zoned> {
    Ok(Timestamp::from_second(seconds)
        .context("invalid Date timestamp")?
        .to_zoned(TimeZone::system()))
}
fn utc_seconds(date: DateTime) -> Result<i64> {
    Ok(TimeZone::UTC.to_timestamp(date)?.as_second())
}
fn normalize(parts: [i32; 6], daylight: i32) -> Result<i64> {
    let date = calendar(parts)?;
    let seconds = utc_seconds(date)? - standard_offset(date.year())? as i64 - daylight as i64;
    ensure!(seconds >= 0, "invalid Date timestamp");
    Ok(seconds)
}

impl Vm {
    pub(crate) fn register_date(&mut self) -> Result<()> {
        let class = self.register_native_class("Date")?;
        for method in [
            "Date",
            "finalize",
            "getYear",
            "getMonth",
            "getDate",
            "getDay",
            "getHours",
            "getMinutes",
            "getSeconds",
            "getTime",
            "setYear",
            "setMonth",
            "setDate",
            "setHours",
            "setMinutes",
            "setSeconds",
            "setTime",
            "parse",
            "getTimezoneOffset",
        ] {
            self.register_native_method(&class, method, &format!("Date.{method}"))?;
        }
        Ok(())
    }

    pub(crate) fn date_call(
        &mut self,
        name: &str,
        context: &Value,
        args: &[Value],
        host: &impl Host,
        result_needed: bool,
    ) -> Result<Value> {
        if name == "getTimezoneOffset" {
            if !result_needed {
                return Ok(Value::Void);
            }
            let now = local(host.unix_time_ms() / 1000)?;
            return Ok(Value::Integer(-(standard_offset(now.year())? as i64) / 60));
        }
        if name == "finalize" {
            return Ok(Value::Void);
        }
        let id = self.object_handle(context)?;
        let old = self.objects[id]
            .date
            .context("Date method requires a Date instance")?;
        let arg = || args.first().context("Date method requires an argument");
        let timestamp = match name {
            "Date" if args.is_empty() => host.unix_time_ms() / 1000,
            "Date" if matches!(args[0], Value::String(_)) => {
                parse(&args[0].text(), host.unix_time_ms())?
            }
            "Date" => {
                let mut parts = [0, 0, 1, 0, 0, 0];
                for (i, value) in args.iter().take(6).enumerate() {
                    if i == 0 || !matches!(value, Value::Void) {
                        parts[i] = value.integer()? as i32;
                    }
                }
                normalize(parts, 0)?
            }
            "setTime" => arg()?.integer()? / 1000,
            "parse" => parse(&arg()?.text(), host.unix_time_ms())?,
            "setYear" | "setMonth" | "setDate" | "setHours" | "setMinutes" | "setSeconds" => {
                let value = arg()?.integer()? as i32;
                let old = local(old)?;
                let mut parts = [
                    old.year() as i32,
                    old.month() as i32 - 1,
                    old.day() as i32,
                    old.hour() as i32,
                    old.minute() as i32,
                    old.second() as i32,
                ];
                let index = match name {
                    "setYear" => 0,
                    "setMonth" => 1,
                    "setDate" => 2,
                    "setHours" => 3,
                    "setMinutes" => 4,
                    _ => 5,
                };
                parts[index] = value;
                match normalize(parts, old.offset().seconds() - standard_offset(old.year())?) {
                    Ok(time) => time,
                    Err(error) => {
                        self.objects[id].date = Some(-1);
                        return Err(error);
                    }
                }
            }
            "getTime" => {
                return Ok(if result_needed {
                    Value::Integer(old.wrapping_mul(1000))
                } else {
                    Value::Void
                });
            }
            "getYear" | "getMonth" | "getDate" | "getDay" | "getHours" | "getMinutes"
            | "getSeconds" => {
                if !result_needed {
                    return Ok(Value::Void);
                }
                let time = local(old)?;
                return Ok(Value::Integer(match name {
                    "getYear" => time.year() as i64,
                    "getMonth" => time.month() as i64 - 1,
                    "getDate" => time.day() as i64,
                    "getDay" => time.weekday().to_sunday_zero_offset() as i64,
                    "getHours" => time.hour() as i64,
                    "getMinutes" => time.minute() as i64,
                    _ => time.second() as i64,
                }));
            }
            _ => bail!("unknown Date method {name}"),
        };
        self.objects[id].date = Some(timestamp);
        Ok(Value::Void)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Token {
    Number(i32),
    Month(i32),
    Weekday,
    Zone(i32),
    Am,
    Pm,
    Punct(u8),
    Comment,
}

fn lex(input: &str) -> Result<Vec<Token>> {
    ensure!(input.len() <= 65536, "Date string exceeds size limit");
    let input = input.split('\0').next().unwrap().to_ascii_lowercase();
    let mut bytes = input.as_bytes();
    let mut tokens = Vec::new();
    while let Some(&c) = bytes.first() {
        if c.is_ascii_whitespace() {
            bytes = &bytes[1..];
            continue;
        }
        if c.is_ascii_digit() {
            let n = bytes.iter().take_while(|c| c.is_ascii_digit()).count();
            let value = bytes[..n].iter().fold(0i32, |v, c| {
                v.wrapping_mul(10).wrapping_add((c - b'0') as i32)
            });
            tokens.push(Token::Number(value));
            bytes = &bytes[n..];
        } else if c.is_ascii_alphabetic() {
            let n = bytes.iter().take_while(|c| c.is_ascii_alphabetic()).count();
            let mut length = n;
            let mut token = word(std::str::from_utf8(&bytes[..n])?);
            if bytes.get(n) == Some(&b'.')
                && let Some(dotted) = word(std::str::from_utf8(&bytes[..n + 1])?)
            {
                token = Some(dotted);
                length += 1;
            }
            tokens.push(token.context("cannot parse Date string")?);
            bytes = &bytes[length..];
        } else if c == b'(' {
            let end = bytes
                .iter()
                .position(|c| *c == b')')
                .context("cannot parse Date comment")?;
            tokens.push(Token::Comment);
            bytes = &bytes[end + 1..];
        } else {
            tokens.push(Token::Punct(c));
            bytes = &bytes[1..];
        }
    }
    Ok(tokens)
}

struct Parser<'a>(&'a [Token]);
impl Parser<'_> {
    fn take(&mut self, token: Token) -> bool {
        if self.0.first() == Some(&token) {
            self.0 = &self.0[1..];
            true
        } else {
            false
        }
    }
    fn number(&mut self) -> Result<i32> {
        let Some(Token::Number(n)) = self.0.first() else {
            bail!("cannot parse Date number");
        };
        let n = *n;
        self.0 = &self.0[1..];
        Ok(n)
    }
    fn time(&mut self) -> Result<[i32; 3]> {
        let before = if self.take(Token::Pm) {
            Some(true)
        } else if self.take(Token::Am) {
            Some(false)
        } else {
            None
        };
        let hour = self.number()?;
        ensure!(self.take(Token::Punct(b':')), "cannot parse Date time");
        let minute = self.number()?;
        let second = if self.take(Token::Punct(b':')) {
            let second = self.number()?;
            if self.take(Token::Punct(b'.')) {
                self.number()?;
            }
            second
        } else {
            0
        };
        let after = if self.take(Token::Pm) {
            Some(true)
        } else if self.take(Token::Am) {
            Some(false)
        } else {
            None
        };
        ensure!(
            before.is_none() || after.is_none(),
            "cannot parse Date AM/PM"
        );
        Ok([
            hour.wrapping_add(if before.or(after) == Some(true) {
                12
            } else {
                0
            }),
            minute,
            second,
        ])
    }
}
fn parse(input: &str, now_ms: i64) -> Result<i64> {
    let tokens = lex(input)?;
    let mut p = Parser(&tokens);
    if p.take(Token::Weekday) {
        p.take(Token::Punct(b','));
    }
    let (mut year, month, day, time);
    // Numeric dates always use year/month/day. Month names also allow the
    // year after the time, as in the original yacc grammar.
    if matches!(
        p.0,
        [
            Token::Number(_),
            Token::Punct(b'-' | b'/'),
            Token::Number(_),
            ..
        ]
    ) {
        year = p.number()?;
        p.0 = &p.0[1..];
        month = p.number()?.wrapping_sub(1);
        ensure!(
            p.take(Token::Punct(b'-')) || p.take(Token::Punct(b'/')),
            "cannot parse Date separator"
        );
        day = p.number()?;
        time = p.time()?;
    } else {
        let separator;
        if let Some(Token::Month(m)) = p.0.first() {
            month = *m;
            p.0 = &p.0[1..];
            separator = p.take(Token::Punct(b'-'));
            day = p.number()?;
        } else {
            day = p.number()?;
            separator = p.take(Token::Punct(b'-'));
            let Some(Token::Month(m)) = p.0.first() else {
                bail!("cannot parse Date month");
            };
            month = *m;
            p.0 = &p.0[1..];
        }
        if p.take(Token::Punct(b'-')) {
            ensure!(separator, "cannot parse Date separator");
            year = p.number()?;
            time = p.time()?;
        } else if matches!(
            p.0,
            [Token::Number(_), Token::Punct(b':'), ..] | [Token::Am | Token::Pm, ..]
        ) {
            time = p.time()?;
            year = p.number()?;
        } else {
            year = p.number()?;
            time = p.time()?;
        }
    }
    if year < 100 {
        year = year.wrapping_add(if year <= 50 { 2000 } else { 1900 });
    }
    let mut zone = None;
    if let Some(Token::Zone(z)) = p.0.first() {
        zone = Some(zone_seconds(*z));
        p.0 = &p.0[1..];
    }
    let sign = if p.take(Token::Punct(b'+')) {
        1
    } else if p.take(Token::Punct(b'-')) {
        -1
    } else {
        0
    };
    if sign != 0 {
        zone = Some(zone.unwrap_or(0) + zone_seconds(p.number()?.wrapping_mul(sign)));
    }
    p.take(Token::Comment);
    ensure!(p.0.is_empty(), "cannot parse Date string");
    let date = calendar([year, month, day, time[0], time[1], time[2]])?;
    ensure!(
        utc_seconds(date)? - standard_offset(date.year())? as i64 >= 0,
        "invalid Date timestamp"
    );
    let offset = zone.unwrap_or(standard_offset(local(now_ms / 1000)?.year())? as i64);
    Ok(utc_seconds(date)? - offset)
}
fn zone_seconds(zone: i32) -> i64 {
    (zone as i64 / 100) * 3600 + (zone as i64 % 100) * 60
}

include!("date_words.rs");
