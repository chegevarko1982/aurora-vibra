//! Определяет попадания по собственному самолёту в потоке /hudmsg.damage[].
//!
//! У API нет структурного способа сказать "это про меня": поля `enemy` и
//! `sender` в `damage[]` подтверждены живым боем как всегда false/пустые
//! (не несут информации), а сам массив — это общий фид повреждений по ВСЕМ
//! игрокам матча, не только по игроку, опрашивающему localhost. Единственный
//! рабочий сигнал — сопоставить свой позывной с текстом `msg`, где он
//! встречается как жертва: `"<атакующий> (<техника>) <глагол> <жертва>
//! (<техника>)"`.

/// Возвращает короткую метку типа попадания, если `msg` описывает урон,
/// нанесённый ИМЕННО позывному `callsign` (а не нанесённый им самим).
/// `callsign` пустой -> детектор выключен, всегда `None`.
///
/// Эвристика по подстроке, а не разбор структуры: имена игроков — свободный
/// текст, поэтому позывной, являющийся суффиксом чужого клан-тега (например
/// `"[ABC]Lazarev_IJN"` при `callsign = "Lazarev_IJN"`), даст ложное
/// совпадение. Компромисс осознанный — то же допущение уже принято в
/// interesting.rs для сопоставления по имени поля.
pub fn classify_own_hit(msg: &str, callsign: &str) -> Option<&'static str> {
    if callsign.is_empty() {
        return None;
    }
    let victim_marker = format!("{callsign} (");
    if !msg.contains(&victim_marker) {
        return None;
    }
    // Если позывной стоит в начале сообщения — это Я нанёс урон, а не
    // получил его; такие сообщения не считаем попаданием по себе.
    if msg.starts_with(&victim_marker) {
        return None;
    }
    if msg.contains("нанёс фатальное повреждение") {
        return Some("фатальное повреждение");
    }
    if msg.contains("нанёс критическое повреждение") {
        return Some("критическое повреждение");
    }
    if msg.contains("сбил") {
        return Some("СБИТ");
    }
    if msg.contains("поджёг") {
        return Some("подожжён");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_fatal_damage_to_self() {
        let msg = "nikkto94 (И-185) нанёс фатальное повреждение Lazarev_IJN (Bf 109 E)";
        assert_eq!(
            classify_own_hit(msg, "Lazarev_IJN"),
            Some("фатальное повреждение")
        );
    }

    #[test]
    fn detects_critical_damage_to_self() {
        let msg = "nikkto94 (И-185) нанёс критическое повреждение Lazarev_IJN (Bf 109 E)";
        assert_eq!(
            classify_own_hit(msg, "Lazarev_IJN"),
            Some("критическое повреждение")
        );
    }

    #[test]
    fn detects_shot_down_self() {
        let msg = "Krill303 (Fw 190 A) сбил Lazarev_IJN (Bf 109 E)";
        assert_eq!(classify_own_hit(msg, "Lazarev_IJN"), Some("СБИТ"));
    }

    #[test]
    fn detects_set_on_fire_self() {
        let msg = "TOTOHIK (Bf 109 F) поджёг Lazarev_IJN (Bf 109 E)";
        assert_eq!(classify_own_hit(msg, "Lazarev_IJN"), Some("подожжён"));
    }

    #[test]
    fn does_not_flag_own_kills_as_hits_on_self() {
        let msg = "Lazarev_IJN (Bf 109 E) сбил TOTOHIK (Bf 109 F)";
        assert_eq!(classify_own_hit(msg, "Lazarev_IJN"), None);
    }

    #[test]
    fn ignores_unrelated_messages() {
        let msg = "canussi (Bf 110 G) сбил ⋇NinjaKid1985 (F4U-4)";
        assert_eq!(classify_own_hit(msg, "Lazarev_IJN"), None);
    }

    #[test]
    fn empty_callsign_disables_detector() {
        let msg = "Krill303 (Fw 190 A) сбил Lazarev_IJN (Bf 109 E)";
        assert_eq!(classify_own_hit(msg, ""), None);
    }

    #[test]
    fn ignores_ground_target_destruction() {
        let msg = "nikkto94 (И-185) уничтожил ПВО";
        assert_eq!(classify_own_hit(msg, "Lazarev_IJN"), None);
    }
}
