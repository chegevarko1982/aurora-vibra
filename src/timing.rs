//! Общая арифметика для развязки "частота тика движка" от "частота, на
//! которой откалибровано экспоненциальное сглаживание (EMA)".
//!
//! И `wt_link::rumble::EngineState::step` (`ENGINE_DRPM_DT_EMA_ALPHA`), и
//! `custom_fx::engine` (`CustomEffect::smoothing_alpha`) применяют EMA-вида
//! `y = y*alpha + x*(1-alpha)` НА КАЖДЫЙ ВЫЗОВ, без учёта того, сколько
//! реального времени (`dt`) прошло между вызовами. Пока движок тикал строго
//! раз в `hid::worker::SEND_INTERVAL` (было 50 мс), это было не важно — `dt`
//! был константой. Как только тик движка развязывается от сетевого опроса и
//! начинает крутиться чаще (см. `wt_link::worker`/`xp_link::worker`),
//! постоянная времени такого сглаживания укорачивается ПРОПОРЦИОНАЛЬНО
//! частоте вызовов — тот же alpha на вдвое чаще идущих тиках сглаживает
//! вдвое слабее. Это ломает откалиброванный на живом железе "подхват"
//! двигателя WT (см. `ENGINE_CATCH_DRPM_DT`) и пользовательские эффекты.
//!
//! [`ema_retain_for_dt`] пересчитывает `alpha` под фактический `dt`, взяв
//! 20 Гц (`dt = 0.05` с) опорной частотой — той, на которой всё уже
//! откалибровано сегодня.

/// Опорный интервал между тиками (20 Гц), на котором откалибровано текущее
/// поведение EMA-сглаживаний в проекте (было `hid::worker::SEND_INTERVAL`
/// до этой задачи).
pub const REFERENCE_DT_S: f64 = 0.05;

/// Пересчитывает коэффициент EMA `alpha` (применяется как
/// `y = y*alpha + x*(1-alpha)` на каждый тик, то есть `alpha` — это ВЕС
/// СТАРОГО значения) под фактический интервал между тиками `dt`, взяв
/// [`REFERENCE_DT_S`] (20 Гц) как опорную частоту.
///
/// Непрерывный аналог EMA — экспоненциальный спад `exp(-dt/tau)`, где `tau`
/// подобрана так, чтобы на опорном интервале `exp(-REFERENCE_DT_S/tau)`
/// совпадал с исходным `alpha`. Отсюда `alpha_eff = alpha ^ (dt / 0.05)`:
/// при `dt == REFERENCE_DT_S` результат совпадает с исходным `alpha` (с
/// точностью до ошибки округления f64); при более частых вызовах (`dt`
/// меньше опорного) `alpha_eff` БЛИЖЕ К 1, то есть каждый отдельный вызов
/// подмешивает МЕНЬШЕ нового значения — ровно чтобы суммарный эффект
/// сглаживания за одно и то же реальное время не изменился при учащении
/// тика; при более редких вызовах — наоборот, ближе к исходному `alpha`
/// меньше (каждый вызов подмешивает больше).
///
/// Вырожденные `dt` не должны ронять вызывающую сторону:
/// - `dt <= 0.0` или НЕ конечен (NaN/±inf) — время не продвинулось (или
///   значение испорчено) — возвращаем `1.0`: старое значение сохраняется
///   целиком, ничего нового не подмешивается ("заморозка" на один тик).
///   Это же значение — естественный предел формулы при `dt -> 0`.
/// - Сколь угодно большой `dt` — результат стремится к `0.0` (за очень
///   долгий интервал EMA обязана почти полностью повторить новое сырое
///   значение, вес старого исчезает), `powf` с большим показателем
///   безопасно уходит в `0.0` в f64 без переполнения/NaN.
///
/// `alpha` тоже клампится в `0.0..=1.0` перед расчётом — на входе может быть
/// как угодно испорченное значение из ручного JSON-импорта пользовательского
/// эффекта.
///
/// Результат всегда лежит в `0.0..=1.0`.
pub fn ema_retain_for_dt(alpha: f64, dt: f64) -> f64 {
    if !dt.is_finite() || dt <= 0.0 {
        return 1.0;
    }
    let alpha = if alpha.is_finite() {
        alpha.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if alpha <= 0.0 {
        return 0.0;
    }
    if alpha >= 1.0 {
        return 1.0;
    }
    let ratio = dt / REFERENCE_DT_S;
    alpha.powf(ratio).clamp(0.0, 1.0)
}

/// То же самое для ВТОРОЙ, противоположной записи EMA, которая встречается в
/// этом проекте: `y += alpha * (x - y)` — здесь `alpha` это вес НОВОГО
/// значения, а не старого (так написан `ENGINE_DRPM_DT_EMA_ALPHA` в
/// `wt_link::rumble::EngineState::step`).
///
/// Разница не косметическая, и на ней уже один раз поймались: на опорной
/// частоте (`dt == REFERENCE_DT_S`) ОБЕ формулы тождественны и возвращают
/// исходный `alpha`, поэтому подмена одной другой не ловится ни одним
/// тестом, который гоняет движок штатным шагом 50 мс — а на учащённом тике
/// расходится в разы И В ПРОТИВОПОЛОЖНУЮ СТОРОНУ: для `alpha = 0.35` при
/// `dt = 0.02` правильный вес нового значения равен 0.158, а если ошибочно
/// позвать [`ema_retain_for_dt`], получится 0.657 — сглаживание не
/// усиливается, а слабеет, причём сильнее, чем вообще без нормировки.
///
/// Поэтому две функции с разными именами, а не одна с параметром: имя
/// обязано называть конвенцию, иначе на месте вызова не видно, какая из них
/// нужна.
///
/// Вырожденные `dt` ведут себя согласованно с [`ema_retain_for_dt`]:
/// `dt <= 0` или не конечен — "заморозка", то есть вес нового значения 0.0.
pub fn ema_blend_for_dt(alpha: f64, dt: f64) -> f64 {
    let alpha = if alpha.is_finite() {
        alpha.clamp(0.0, 1.0)
    } else {
        0.0
    };
    1.0 - ema_retain_for_dt(1.0 - alpha, dt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_dt_returns_alpha_unchanged() {
        for alpha in [0.0, 0.1, 0.35, 0.5, 0.9, 0.99, 1.0] {
            let got = ema_retain_for_dt(alpha, REFERENCE_DT_S);
            assert!(
                (got - alpha).abs() < 1e-12,
                "alpha={alpha} dt=REFERENCE_DT_S должно вернуть alpha без изменений, got={got}"
            );
        }
    }

    #[test]
    fn degenerate_dt_stays_in_unit_range() {
        for dt in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 10.0] {
            for alpha in [0.0, 0.35, 0.9, 1.0] {
                let got = ema_retain_for_dt(alpha, dt);
                assert!(
                    (0.0..=1.0).contains(&got) && got.is_finite(),
                    "alpha={alpha} dt={dt} -> {got} вне 0..=1"
                );
            }
        }
    }

    #[test]
    fn zero_or_negative_dt_freezes() {
        assert_eq!(ema_retain_for_dt(0.35, 0.0), 1.0);
        assert_eq!(ema_retain_for_dt(0.35, -0.01), 1.0);
        assert_eq!(ema_retain_for_dt(0.35, f64::NAN), 1.0);
    }

    #[test]
    fn huge_dt_approaches_zero() {
        // Долгий интервал — почти весь вес уходит новому сырому значению
        // (вес старого, `alpha_eff`, почти нулевой).
        let got = ema_retain_for_dt(0.35, 1_000_000.0);
        assert!(got < 1e-6, "got={got}");
    }

    /// Смысл нормировки: два тика по 0.025с должны давать примерно тот же
    /// суммарный эффект сглаживания, что один тик по 0.05с (опорная частота).
    #[test]
    fn two_half_steps_match_one_reference_step() {
        let alpha = 0.35;
        let raw = 1.0_f64;

        // Один шаг на опорной частоте.
        let mut ema_ref = 0.0;
        let a_ref = ema_retain_for_dt(alpha, REFERENCE_DT_S);
        ema_ref = ema_ref * a_ref + raw * (1.0 - a_ref);

        // Два шага вдвое чаще (учащённый тик движка).
        let mut ema_fast = 0.0;
        let a_fast = ema_retain_for_dt(alpha, REFERENCE_DT_S / 2.0);
        ema_fast = ema_fast * a_fast + raw * (1.0 - a_fast);
        ema_fast = ema_fast * a_fast + raw * (1.0 - a_fast);

        assert!(
            (ema_ref - ema_fast).abs() < 1e-9,
            "ema_ref={ema_ref} ema_fast={ema_fast} должны почти совпасть"
        );
    }

    /// Тот же смысл, но для ВТОРОЙ конвенции (`y += alpha * (x - y)`,
    /// `alpha` — вес нового значения). Именно этот тест ловит подмену
    /// `ema_blend_for_dt` на `ema_retain_for_dt`: на опорной частоте обе
    /// тождественны, поэтому регрессионные тесты движка WT, гоняющие его
    /// штатным шагом 0.05с, такую подмену пропускают.
    #[test]
    fn blend_two_half_steps_match_one_reference_step() {
        let alpha = 0.35;
        let raw = 1.0_f64;

        let mut ema_ref = 0.0;
        let a_ref = ema_blend_for_dt(alpha, REFERENCE_DT_S);
        ema_ref += a_ref * (raw - ema_ref);

        let mut ema_fast = 0.0;
        let a_fast = ema_blend_for_dt(alpha, REFERENCE_DT_S / 2.0);
        ema_fast += a_fast * (raw - ema_fast);
        ema_fast += a_fast * (raw - ema_fast);

        assert!(
            (ema_ref - ema_fast).abs() < 1e-9,
            "ema_ref={ema_ref} ema_fast={ema_fast} должны почти совпасть"
        );
    }

    /// Направление, а не только опорная точка: у конвенции "вес нового"
    /// учащение тика обязано УМЕНЬШАТЬ вклад одного вызова. Ошибочный вызов
    /// `ema_retain_for_dt` дал бы здесь 0.657 вместо 0.158 — то есть больше,
    /// чем даже нормировки вовсе (0.35), и тест бы упал.
    #[test]
    fn blend_weakens_each_tick_when_ticking_faster() {
        let alpha = 0.35;
        let a_fast = ema_blend_for_dt(alpha, 0.02);
        assert!(
            a_fast < alpha,
            "вес нового значения на учащённом тике ({a_fast}) обязан быть МЕНЬШЕ исходного {alpha}"
        );
        assert!(
            (a_fast - 0.158_310).abs() < 1e-4,
            "ожидали ~0.1583, получили {a_fast}"
        );
    }

    #[test]
    fn blend_and_retain_are_mirror_conventions() {
        for dt in [0.005, 0.02, REFERENCE_DT_S, 0.2] {
            for alpha in [0.0, 0.18, 0.35, 0.9, 1.0] {
                let blend = ema_blend_for_dt(alpha, dt);
                let retain = ema_retain_for_dt(1.0 - alpha, dt);
                assert!(
                    (blend - (1.0 - retain)).abs() < 1e-12,
                    "alpha={alpha} dt={dt}: blend={blend}, 1-retain={}",
                    1.0 - retain
                );
            }
        }
    }

    #[test]
    fn blend_degenerate_dt_freezes() {
        for dt in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let got = ema_blend_for_dt(0.35, dt);
            assert_eq!(got, 0.0, "dt={dt}: заморозка = нулевой вес нового значения");
        }
    }

    #[test]
    fn smaller_dt_gives_alpha_eff_closer_to_one() {
        // Более частый тик (меньший dt) должен давать БОЛЬШИЙ эффективный
        // alpha за один вызов — иначе за то же реальное время сглаживание
        // ослабнет (см. doc-комментарий выше).
        let alpha = 0.35;
        let a_fast = ema_retain_for_dt(alpha, 0.02);
        let a_slow = ema_retain_for_dt(alpha, 0.05);
        assert!(a_fast > a_slow);
    }
}
