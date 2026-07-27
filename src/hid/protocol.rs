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

/// Джойстик (sidestick) — любой известный вариант L/R (Airbus/Fighter/Space).
/// ВАЖНО: используется вместо "не РУД" (`!is_ursa_minor_throttle`) там, где
/// нужно определить "это именно джойстик" — VID 0x4098 у WinWing общий для
/// РАЗНЫХ устройств (МФД, панели и т.д.), поэтому "не РУД" ошибочно засчитывал
/// бы любое другое стороннее устройство WinWing как подключённый джойстик.
pub fn is_ursa_minor_joystick(pid: u16) -> bool {
    is_ursa_minor_left(pid) || is_ursa_minor_right(pid)
}

/// Байт адреса вибро-канала джойстика.
///
/// Подтверждено USB-снифом (Wireshark): Airbus L/R и Fighter R сняты и
/// сверены напрямую с автором апстрима (rtroncoso/ursa-minor-ffb) на
/// реальном железе; Fighter L и Space L/R получены по той же снятой схеме
/// адресации (см. апстрим-коммит "fix: support all sidestick variant types").
///
/// Схема: каждый вариант занимает свою пару соседних байт —
/// Airbus 0x07/0x08, Fighter 0x09/0x0A, Space 0x0B/0x0C.
pub fn channel_byte_for_pid(pid: u16) -> u8 {
    match pid {
        WW_PID_URSA_MINOR_AIRBUS_L => 0x07,
        WW_PID_URSA_MINOR_AIRBUS_R => 0x08,
        WW_PID_URSA_MINOR_FIGHTER_L => 0x09,
        WW_PID_URSA_MINOR_FIGHTER_R => 0x0A,
        WW_PID_URSA_MINOR_SPACE_L => 0x0B,
        WW_PID_URSA_MINOR_SPACE_R => 0x0C,
        _ => 0x07, // безопасный дефолт для неизвестного PID
    }
}

pub fn build_simapp_vibe_frame(pid: u16, report_id: u8, out_len: u16, intensity: u8) -> Vec<u8> {
    let handed_selector = channel_byte_for_pid(pid);

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
pub fn build_throttle_vibe_frame(
    report_id: u8,
    out_len: u16,
    motor_addr: u8,
    intensity: u8,
) -> Vec<u8> {
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
            &[
                0x07, 0xBF, 0x00, 0x00, 0x03, 0x49, 0x00, 0x19, 0, 0, 0, 0, 0
            ]
        );
    }

    #[test]
    fn airbus_right_uses_handed_byte_08() {
        let frame = build_simapp_vibe_frame(WW_PID_URSA_MINOR_AIRBUS_R, 0x02, 14, 0x19);
        assert_eq!(frame[1], 0x08);
        assert_eq!(frame[8], 0x19);
    }

    #[test]
    fn all_pids_have_correct_channel_byte() {
        assert_eq!(channel_byte_for_pid(WW_PID_URSA_MINOR_AIRBUS_L), 0x07);
        assert_eq!(channel_byte_for_pid(WW_PID_URSA_MINOR_AIRBUS_R), 0x08);
        assert_eq!(channel_byte_for_pid(WW_PID_URSA_MINOR_FIGHTER_L), 0x09);
        assert_eq!(channel_byte_for_pid(WW_PID_URSA_MINOR_FIGHTER_R), 0x0A);
        assert_eq!(channel_byte_for_pid(WW_PID_URSA_MINOR_SPACE_L), 0x0B);
        assert_eq!(channel_byte_for_pid(WW_PID_URSA_MINOR_SPACE_R), 0x0C);
    }

    #[test]
    fn unknown_pid_defaults_to_airbus_left_channel() {
        assert_eq!(channel_byte_for_pid(0xFFFF), 0x07);
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
            vec![
                0x02, 0x10, 0xB9, 0x00, 0x00, 0x03, 0x49, 0x0E, 0x7F, 0, 0, 0, 0, 0
            ]
        );
    }

    #[test]
    fn throttle_right_motor_frame_matches_golden_bytes() {
        // Golden bytes из test_vibroTH.rs: buf[7]=0x10 (правый мотор)
        let frame = build_throttle_vibe_frame(0x02, 14, THROTTLE_MOTOR_RIGHT, 0xFF);
        assert_eq!(
            frame,
            vec![
                0x02, 0x10, 0xB9, 0x00, 0x00, 0x03, 0x49, 0x10, 0xFF, 0, 0, 0, 0, 0
            ]
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
