//! Эвристический поиск "интересных для тактильных эффектов величин" среди
//! УЖЕ обнаруженных схемой ключей (см. schema.rs). Это не хардкод контракта
//! игры — сопоставление идёт по подстрокам в реально встретившихся именах
//! ключей, и если совпадения нет, категория так и помечается "не найдено".
//! (В отличие от weapons.rs, где детект оружия — чисто по поведению: здесь
//! речь не про автоматическое обнаружение неизвестной схемы, а про удобный
//! для оператора обзор, поэтому эвристика по имени уместна и явно
//! декларируется как best-effort.)

pub struct Category {
    pub name: &'static str,
    pub patterns: &'static [&'static str],
}

pub const CATEGORIES: &[Category] = &[
    Category {
        name: "Обороты двигателя (RPM)",
        patterns: &["rpm"],
    },
    Category {
        name: "Наддув/турбо",
        patterns: &["manifold", "boost", "turbo", "mp0"],
    },
    Category {
        name: "РУД (throttle)",
        patterns: &["throttle"],
    },
    Category {
        name: "Температура двигателя",
        // Реальные ключи War Thunder используют пробел, не подчёркивание
        // (подтверждено живым захватом: "oil temp 1, C", "water temp 1, C").
        patterns: &["oil temp", "water temp", "cht", "egt", "temperature"],
    },
    Category {
        name: "Отказ/пожар двигателя",
        patterns: &["fire", "engine_fail", "broken", "damaged"],
    },
    Category {
        name: "Шасси",
        patterns: &["gear"],
    },
    Category {
        name: "Закрылки",
        patterns: &["flap"],
    },
    Category {
        name: "Воздушный тормоз/спойлер",
        patterns: &["airbrake", "air_brake", "dive_brake", "spoiler"],
    },
    Category {
        name: "Тормоза колёс",
        patterns: &["wheel_brake", "brake"],
    },
    Category {
        name: "Скорость (IAS/TAS/Mach)",
        patterns: &["ias", "tas", "mach", "speed"],
    },
    Category {
        name: "Высота",
        patterns: &["altitude", "alt_h"],
    },
    Category {
        name: "Вертикальная скорость",
        patterns: &["vy", "vertical_speed", "climb"],
    },
    Category {
        name: "Крен",
        patterns: &["roll", "bank"],
    },
    Category {
        name: "Тангаж",
        patterns: &["pitch"],
    },
    Category {
        name: "Угол атаки",
        patterns: &["aoa", "angle_of_attack"],
    },
    Category {
        name: "Скольжение",
        patterns: &["aos", "sideslip", "slip"],
    },
    Category {
        name: "Перегрузка (G)",
        patterns: &["ny", "nx", "nz", "g_meter", "gforce", "g_force"],
    },
    Category {
        name: "Срыв/сваливание",
        patterns: &["stall", "buffet", "shake"],
    },
    Category {
        name: "Боеприпасы (по имени)",
        patterns: &[
            "ammo", "shell", "cannon", "mgun", "rocket", "bomb", "torpedo", "weapon",
        ],
    },
    Category {
        name: "Контакт с землёй",
        patterns: &["ground", "wow", "weight_on_wheel"],
    },
    Category {
        name: "Тип техники",
        patterns: &["type"],
    },
];

/// Русское название категории для одного ключа, если она распознана
/// (case-insensitive подстрока). Используется панелью "все поля" на
/// дашборде, чтобы расшифровывать каждое обнаруженное поле сразу.
pub fn decode_key(key: &str) -> Option<&'static str> {
    let lk = key.to_lowercase();
    CATEGORIES
        .iter()
        .find(|cat| cat.patterns.iter().any(|p| lk.contains(p)))
        .map(|cat| cat.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_key_finds_category_for_known_field() {
        assert_eq!(decode_key("RPM 1"), Some("Обороты двигателя (RPM)"));
        assert_eq!(decode_key("manifold pressure 1, atm"), Some("Наддув/турбо"));
    }

    #[test]
    fn decode_key_none_for_unknown_field() {
        assert_eq!(decode_key("totally_unrelated_field"), None);
    }
}
