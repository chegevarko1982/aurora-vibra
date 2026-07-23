pub const WW_VID: u16 = 0x4098;

pub const WW_PID_URSA_MINOR_AIRBUS_L: u16 = 0xBC27;
pub const WW_PID_URSA_MINOR_AIRBUS_R: u16 = 0xBC28;
pub const WW_PID_URSA_MINOR_FIGHTER_L: u16 = 0xBC29;
pub const WW_PID_URSA_MINOR_FIGHTER_R: u16 = 0xBC2A;
pub const WW_PID_URSA_MINOR_SPACE_L: u16 = 0xBC2B;
pub const WW_PID_URSA_MINOR_SPACE_R: u16 = 0xBC2C;

// WINCTRL URSA MINOR Throttle (РУД). В отличие от джойстиков (Combat/Airbus/
// Space L/R), это ОТДЕЛЬНОЕ физическое устройство с двумя своими
// вибромоторами (левый/правый), адресуемыми одним и тем же HID-репортом
// через разные значения байта адреса мотора (см. THROTTLE_MOTOR_LEFT/RIGHT).
pub const WW_PID_URSA_MINOR_THROTTLE: u16 = 0xB920;

// Адреса моторов в протоколе Throttle (buf[7] в 14-байтовом Output Report).
pub const THROTTLE_MOTOR_LEFT: u8 = 0x0E;
pub const THROTTLE_MOTOR_RIGHT: u8 = 0x10;

pub fn ursa_model_name(pid: u16) -> &'static str {
    match pid {
        WW_PID_URSA_MINOR_AIRBUS_L => "URSA MINOR AIRBUS L",
        WW_PID_URSA_MINOR_AIRBUS_R => "URSA MINOR AIRBUS R",
        WW_PID_URSA_MINOR_FIGHTER_L => "URSA MINOR FIGHTER L",
        WW_PID_URSA_MINOR_FIGHTER_R => "URSA MINOR FIGHTER R",
        WW_PID_URSA_MINOR_SPACE_L => "URSA MINOR SPACE L",
        WW_PID_URSA_MINOR_SPACE_R => "URSA MINOR SPACE R",
        WW_PID_URSA_MINOR_THROTTLE => "URSA MINOR THROTTLE",
        _ => "UNKNOWN",
    }
}

/// РУД (WINCTRL URSA MINOR Throttle) — отдельное устройство, не джойстик.
pub fn is_ursa_minor_throttle(pid: u16) -> bool {
    pid == WW_PID_URSA_MINOR_THROTTLE
}

pub fn is_ursa_minor_left(pid: u16) -> bool {
    matches!(
        pid,
        WW_PID_URSA_MINOR_AIRBUS_L | WW_PID_URSA_MINOR_FIGHTER_L | WW_PID_URSA_MINOR_SPACE_L
    )
}

pub fn is_ursa_minor_right(pid: u16) -> bool {
    matches!(
        pid,
        WW_PID_URSA_MINOR_AIRBUS_R | WW_PID_URSA_MINOR_FIGHTER_R | WW_PID_URSA_MINOR_SPACE_R
    )
}

pub fn handed_selector_for_pid(pid: u16) -> u8 {
    // --- НАШ ХАК ДЛЯ COMBAT ВЕРСИИ ---
    // Если подключен Fighter R (PID 0xBC2A), отдаем перехваченный байт 0x0A
    if pid == WW_PID_URSA_MINOR_FIGHTER_R {
        return 0x0A;
    }

    // Для всех остальных джойстиков сохраняем оригинальную логику
    if is_ursa_minor_right(pid) {
        0x08
    } else if is_ursa_minor_left(pid) {
        0x07
    } else {
        0x07
    }
}

pub fn build_simapp_vibe_frame(pid: u16, report_id: u8, out_len: u16, intensity: u8) -> Vec<u8> {
    let handed_selector = handed_selector_for_pid(pid);

    let body: [u8; 13] = [
        handed_selector,
        0xBF,
        0x00,
        0x00,
        0x03,
        0x49,
        0x00,
        intensity,
        0,
        0,
        0,
        0,
        0,
    ];

    let len = out_len as usize;
    let mut buf = vec![0u8; len];

    if len == 0 {
        return buf;
    }

    buf[0] = report_id;
    let copy_len = body.len().min(len.saturating_sub(1));
    buf[1..1 + copy_len].copy_from_slice(&body[..copy_len]);
    buf
}

/// Собирает Output Report для WINCTRL URSA MINOR Throttle (РУД).
///
/// Формат (см. test_vibroTH.rs, подтверждённый живым тестом на железе):
/// `[report_id, 0x10, 0xB9, 0x00, 0x00, 0x03, 0x49, motor_addr, intensity, 0,0,0,0,0]`
/// где motor_addr — THROTTLE_MOTOR_LEFT (0x0E) или THROTTLE_MOTOR_RIGHT (0x10).
/// В отличие от джойстиков, заголовок здесь фиксирован и не зависит от PID —
/// это отдельный протокол устройства.
pub fn build_throttle_vibe_frame(report_id: u8, out_len: u16, motor_addr: u8, intensity: u8) -> Vec<u8> {
    let body: [u8; 13] = [
        0x10, 0xB9, 0x00, 0x00, 0x03, 0x49, motor_addr, intensity, 0, 0, 0, 0, 0,
    ];

    let len = out_len as usize;
    let mut buf = vec![0u8; len];

    if len == 0 {
        return buf;
    }

    buf[0] = report_id;
    let copy_len = body.len().min(len.saturating_sub(1));
    buf[1..1 + copy_len].copy_from_slice(&body[..copy_len]);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn airbus_left_frame_matches_golden_bytes() {
        let frame = build_simapp_vibe_frame(WW_PID_URSA_MINOR_AIRBUS_L, 0x02, 14, 0x19);
        assert_eq!(frame.len(), 14);
        assert_eq!(frame[0], 0x02);
        assert_eq!(
            &frame[1..],
            &[0x07, 0xBF, 0x00, 0x00, 0x03, 0x49, 0x00, 0x19, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn airbus_right_uses_handed_byte_08() {
        let frame = build_simapp_vibe_frame(WW_PID_URSA_MINOR_AIRBUS_R, 0x02, 14, 0x19);
        assert_eq!(frame[1], 0x08);
        assert_eq!(frame[8], 0x19);
    }

    #[test]
    fn all_pids_have_correct_handed_selector() {
        let left_pids = [
            WW_PID_URSA_MINOR_AIRBUS_L,
            WW_PID_URSA_MINOR_FIGHTER_L,
            WW_PID_URSA_MINOR_SPACE_L,
        ];
        // Fighter R (0xBC2A) — особый случай: хак для Combat-версии прошивки,
        // отдаёт перехваченный байт 0x0A вместо обычного 0x08.
        let right_pids = [WW_PID_URSA_MINOR_AIRBUS_R, WW_PID_URSA_MINOR_SPACE_R];

        for pid in left_pids {
            assert_eq!(handed_selector_for_pid(pid), 0x07, "pid=0x{pid:04X}");
        }
        for pid in right_pids {
            assert_eq!(handed_selector_for_pid(pid), 0x08, "pid=0x{pid:04X}");
        }
        assert_eq!(
            handed_selector_for_pid(WW_PID_URSA_MINOR_FIGHTER_R),
            0x0A,
            "pid=0x{WW_PID_URSA_MINOR_FIGHTER_R:04X} (Combat hack)"
        );
    }

    #[test]
    fn unknown_pid_defaults_to_left_selector() {
        assert_eq!(handed_selector_for_pid(0xFFFF), 0x07);
    }

    #[test]
    fn frame_truncates_when_out_len_is_short() {
        let frame = build_simapp_vibe_frame(WW_PID_URSA_MINOR_AIRBUS_L, 0x02, 4, 0x50);
        assert_eq!(frame, vec![0x02, 0x07, 0xBF, 0x00]);
    }

    #[test]
    fn zero_out_len_returns_empty_buffer() {
        assert!(build_simapp_vibe_frame(WW_PID_URSA_MINOR_AIRBUS_L, 0x02, 0, 0x80).is_empty());
    }

    #[test]
    fn intensity_byte_is_at_body_offset_seven() {
        for intensity in [0u8, 1, 127, 255] {
            let frame = build_simapp_vibe_frame(WW_PID_URSA_MINOR_FIGHTER_L, 0x02, 14, intensity);
            assert_eq!(frame[8], intensity);
        }
    }

    #[test]
    fn model_names_for_known_pids() {
        assert_eq!(
            ursa_model_name(WW_PID_URSA_MINOR_AIRBUS_L),
            "URSA MINOR AIRBUS L"
        );
        assert_eq!(
            ursa_model_name(WW_PID_URSA_MINOR_SPACE_R),
            "URSA MINOR SPACE R"
        );
        assert_eq!(ursa_model_name(0x0000), "UNKNOWN");
        assert_eq!(
            ursa_model_name(WW_PID_URSA_MINOR_THROTTLE),
            "URSA MINOR THROTTLE"
        );
    }

    #[test]
    fn throttle_pid_is_recognized() {
        assert!(is_ursa_minor_throttle(WW_PID_URSA_MINOR_THROTTLE));
        assert!(!is_ursa_minor_throttle(WW_PID_URSA_MINOR_AIRBUS_L));
    }

    #[test]
    fn throttle_left_motor_frame_matches_golden_bytes() {
        // Golden bytes из test_vibroTH.rs: buf[7]=0x0e (левый мотор)
        let frame = build_throttle_vibe_frame(0x02, 14, THROTTLE_MOTOR_LEFT, 0x7F);
        assert_eq!(
            frame,
            vec![0x02, 0x10, 0xB9, 0x00, 0x00, 0x03, 0x49, 0x0E, 0x7F, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn throttle_right_motor_frame_matches_golden_bytes() {
        // Golden bytes из test_vibroTH.rs: buf[7]=0x10 (правый мотор)
        let frame = build_throttle_vibe_frame(0x02, 14, THROTTLE_MOTOR_RIGHT, 0xFF);
        assert_eq!(
            frame,
            vec![0x02, 0x10, 0xB9, 0x00, 0x00, 0x03, 0x49, 0x10, 0xFF, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn throttle_frame_truncates_when_out_len_is_short() {
        let frame = build_throttle_vibe_frame(0x02, 4, THROTTLE_MOTOR_LEFT, 0x50);
        assert_eq!(frame, vec![0x02, 0x10, 0xB9, 0x00]);
    }

    #[test]
    fn throttle_zero_out_len_returns_empty_buffer() {
        assert!(build_throttle_vibe_frame(0x02, 0, THROTTLE_MOTOR_LEFT, 0x80).is_empty());
    }
}
