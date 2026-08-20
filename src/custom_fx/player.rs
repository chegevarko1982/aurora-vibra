//! Загрузка записанных сессий с диска для проигрывания в редакторе кастомных эффектов.
//! Позволяет настраивать и отлаживать эффекты по реальным вылетам без запущенной игры.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::custom_fx::sources::TelemetryFrame;
use crate::types::FlightVars;
use crate::wt_link::vars;

/// Один кадр записанной сессии: момент времени от начала записи и телеметрия.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionFrame {
    pub t: f64,
    pub frame: TelemetryFrame,
}

/// Загруженная запись целиком.
#[derive(Debug, Clone)]
pub struct Session {
    pub frames: Vec<SessionFrame>,
    pub source_path: PathBuf,
    /// Временной интервал записи (t последнего кадра минус t первого).
    /// Равна 0.0 для пустой записи.
    pub duration_s: f64,
}

/// Загрузить записанную сессию с диска.
///
/// Формат: JSONL с записями `{"t":<f64>,"endpoint":"state"|"indicators"|"flightvars","body":{...}}`
///
/// Правила разбора:
/// - Битые строки молча пропускаются (записи обрываются на аварийном завершении).
/// - `endpoint == "flightvars"` → `TelemetryFrame::Flight` (разбирается через serde_json).
/// - `endpoint == "state"` / `"indicators"` → собираются в пару для одного WT-кадра.
///   Кадр выпускается, когда `t` меняется или в конце файла. Недостающее тело подменяется
///   пустым объектом `{}`, что безопасно для `wt_link::vars::parse` (отсутствующие поля
///   самонейтрализуются в 0.0/false).
/// - Смешанные записи (flightvars и state/indicators в одном файле) допустимы.
/// - Пустой файл → `Ok(Session { frames: [], duration_s: 0.0, .. })`.
pub fn load_session(path: &Path) -> std::io::Result<Session> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut frames = Vec::new();
    let mut last_state: Option<Value> = None;
    let mut last_indicators: Option<Value> = None;
    let mut last_t: Option<f64> = None;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            // Обрыв записи на аварийном завершении процесса — штатная
            // ситуация, уже прочитанные кадры терять нельзя.
            Err(_) => continue,
        };

        if line.trim().is_empty() {
            continue;
        }

        let parsed: RawLine = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // WT пишет ДВЕ строки на тик (state и indicators) с одинаковым t, см.
        // wt_link::recorder — поэтому кадр собирается не по строке, а по смене t.
        if let Some(prev_t) = last_t
            && (parsed.t - prev_t).abs() > 1e-9
            && (last_state.is_some() || last_indicators.is_some())
        {
            let state = last_state.take().unwrap_or_else(|| json!({}));
            let indicators = last_indicators.take().unwrap_or_else(|| json!({}));
            let wt_frame = vars::parse(prev_t, &state, &indicators);
            frames.push(SessionFrame {
                t: prev_t,
                frame: TelemetryFrame::Wt(wt_frame),
            });
        }

        match parsed.endpoint.as_str() {
            "flightvars" => {
                if let Ok(flight_vars) = serde_json::from_value::<FlightVars>(parsed.body) {
                    frames.push(SessionFrame {
                        t: parsed.t,
                        frame: TelemetryFrame::Flight(flight_vars),
                    });
                }
            }
            "state" => {
                last_t = Some(parsed.t);
                last_state = Some(parsed.body);
            }
            "indicators" => {
                last_t = Some(parsed.t);
                last_indicators = Some(parsed.body);
            }
            // Незнакомый endpoint — запись сделана более новой версией: молча
            // пропускаем, а не роняем загрузку.
            _ => {}
        }
    }

    // Хвост записи: последний тик уже не догонит смена t.
    if let Some(t) = last_t
        && (last_state.is_some() || last_indicators.is_some())
    {
        let state = last_state.unwrap_or_else(|| json!({}));
        let indicators = last_indicators.unwrap_or_else(|| json!({}));
        let wt_frame = vars::parse(t, &state, &indicators);
        frames.push(SessionFrame {
            t,
            frame: TelemetryFrame::Wt(wt_frame),
        });
    }

    let duration_s = if frames.is_empty() {
        0.0
    } else {
        frames.last().unwrap().t - frames.first().unwrap().t
    };

    Ok(Session {
        frames,
        source_path: path.to_path_buf(),
        duration_s,
    })
}

/// Каталог, куда пишет встроенный рекордер (для UI, чтобы открывать
/// файловый диалог сразу в нужном месте).
pub fn sessions_dir() -> PathBuf {
    // Тот же приоритет путей, что у `wt_link::recorder::sessions_dir` —
    // сначала рядом с exe (портативная установка), потом %LOCALAPPDATA%,
    // потом %TEMP%.
    if let Ok(p) = std::env::current_exe()
        && let Some(dir) = p.parent()
    {
        return dir.join("wt_probe_sessions");
    }
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        let mut p = PathBuf::from(base);
        p.push("AuroraVibra");
        p.push("wt_probe_sessions");
        return p;
    }
    std::env::temp_dir().join("wt_probe_sessions")
}

/// Внутренняя структура для разбора JSONL-строки
#[derive(Debug)]
struct RawLine {
    t: f64,
    endpoint: String,
    body: Value,
}

impl<'de> serde::Deserialize<'de> for RawLine {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::MapAccess;

        struct RawLineVisitor;

        impl<'de> serde::de::Visitor<'de> for RawLineVisitor {
            type Value = RawLine;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("объект с полями t, endpoint, body")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut t = None;
                let mut endpoint = None;
                let mut body = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        "t" => t = Some(map.next_value()?),
                        "endpoint" => endpoint = Some(map.next_value()?),
                        "body" => body = Some(map.next_value()?),
                        _ => {
                            let _ = map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }

                let t = t.ok_or_else(|| serde::de::Error::missing_field("t"))?;
                let endpoint =
                    endpoint.ok_or_else(|| serde::de::Error::missing_field("endpoint"))?;
                let body = body.unwrap_or(json!({}));

                Ok(RawLine { t, endpoint, body })
            }
        }

        deserializer.deserialize_map(RawLineVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_file(suffix: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("aurora_vibra_test_{}.jsonl", suffix));
        path
    }

    #[test]
    fn load_wt_state_and_indicators_pair() {
        let path = temp_file("wt_pair");
        let mut f = std::fs::File::create(&path).unwrap();
        // Одна пара state+indicators с одинаковым t
        writeln!(
            f,
            "{{\"t\":1.5,\"endpoint\":\"state\",\"body\":{{\"valid\":true,\"flaps, %\":33,\"IAS, km/h\":380}}}}"
        )
        .unwrap();
        writeln!(
            f,
            "{{\"t\":1.5,\"endpoint\":\"indicators\",\"body\":{{\"weapon1\":1.0}}}}"
        )
        .unwrap();

        let session = load_session(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(session.frames.len(), 1);
        assert_eq!(session.frames[0].t, 1.5);
        assert_eq!(session.duration_s, 0.0); // Только один кадр

        match &session.frames[0].frame {
            TelemetryFrame::Wt(wv) => {
                assert_eq!(wv.t, 1.5);
                assert!(wv.in_mission);
                assert!(wv.weapon1_firing);
                assert_eq!(wv.flaps_pct, 33.0);
            }
            _ => panic!("Ожидался WT-кадр"),
        }
    }

    #[test]
    fn load_flightvars_frame() {
        let path = temp_file("flight_frame");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            "{{\"t\":2.0,\"endpoint\":\"flightvars\",\"body\":{{\"airspeed_indicated\":250.0,\"on_ground\":false}}}}"
        )
        .unwrap();

        let session = load_session(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(session.frames.len(), 1);
        assert_eq!(session.frames[0].t, 2.0);

        match &session.frames[0].frame {
            TelemetryFrame::Flight(fv) => {
                assert_eq!(fv.airspeed_indicated, 250.0);
                assert!(!fv.on_ground);
            }
            _ => panic!("Ожидался Flight-кадр"),
        }
    }

    #[test]
    fn skip_malformed_lines() {
        let path = temp_file("malformed");
        let mut f = std::fs::File::create(&path).unwrap();
        // Валидная строка
        writeln!(f, "{{\"t\":1.0,\"endpoint\":\"state\",\"body\":{{}}}}").unwrap();
        // Битая (не JSON)
        writeln!(f, "not json at all").unwrap();
        // Ещё одна валидная
        writeln!(f, "{{\"t\":1.0,\"endpoint\":\"indicators\",\"body\":{{}}}}").unwrap();

        let session = load_session(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        // Должен загрузиться один WT-кадр (из двух валидных строк)
        assert_eq!(session.frames.len(), 1);
    }

    #[test]
    fn empty_file_is_ok() {
        let path = temp_file("empty");
        let _f = std::fs::File::create(&path).unwrap();

        let session = load_session(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(session.frames.len(), 0);
        assert_eq!(session.duration_s, 0.0);
    }

    #[test]
    fn duration_calculated_from_first_and_last_frame() {
        let path = temp_file("duration");
        let mut f = std::fs::File::create(&path).unwrap();
        // Кадр в t=1.0
        writeln!(f, "{{\"t\":1.0,\"endpoint\":\"state\",\"body\":{{}}}}").unwrap();
        writeln!(f, "{{\"t\":1.0,\"endpoint\":\"indicators\",\"body\":{{}}}}").unwrap();
        // Кадр в t=5.5
        writeln!(f, "{{\"t\":5.5,\"endpoint\":\"state\",\"body\":{{}}}}").unwrap();
        writeln!(f, "{{\"t\":5.5,\"endpoint\":\"indicators\",\"body\":{{}}}}").unwrap();

        let session = load_session(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(session.frames.len(), 2);
        assert_eq!(session.frames[0].t, 1.0);
        assert_eq!(session.frames[1].t, 5.5);
        assert!((session.duration_s - 4.5).abs() < 1e-9);
    }

    #[test]
    fn incomplete_wt_pair_finalized_at_eof() {
        let path = temp_file("incomplete_pair");
        let mut f = std::fs::File::create(&path).unwrap();
        // Только state, indicators отсутствует
        writeln!(
            f,
            "{{\"t\":2.0,\"endpoint\":\"state\",\"body\":{{\"valid\":true,\"flaps, %\":50}}}}"
        )
        .unwrap();

        let session = load_session(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        // Должен создать WT-кадр, подставив пустой indicators
        assert_eq!(session.frames.len(), 1);
        assert_eq!(session.frames[0].t, 2.0);

        match &session.frames[0].frame {
            TelemetryFrame::Wt(wv) => {
                assert!(wv.in_mission);
                assert_eq!(wv.flaps_pct, 50.0);
                assert!(!wv.weapon1_firing); // Пустой indicators
            }
            _ => panic!("Ожидался WT-кадр"),
        }
    }
}
