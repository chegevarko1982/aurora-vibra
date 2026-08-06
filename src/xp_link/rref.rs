//! UDP-транспорт штатного протокола X-Plane "RREF" (Request DataRef) —
//! бинарная подписка на datarefs по UDP на порту 49000, без HTTP/websocket
//! посредников (в отличие от War Thunder, у X-Plane нет HTTP API телеметрии).
//!
//! Кодек (`encode_request`/`decode_packet`) вынесен в свободные функции,
//! чтобы гонять его в юнит-тестах без реального сокета — сеть здесь только
//! у `RrefClient`.

use std::io::ErrorKind;
use std::net::UdpSocket;
use std::thread;
use std::time::Duration;

use super::datarefs::DrIdx;

/// Заголовок пакета запроса подписки, всегда 5 байт (с NUL-терминатором).
const REQUEST_MAGIC: &[u8; 5] = b"RREF\0";
/// Ровно 400 байт под имя dataref, NUL-паддинг — фиксированный формат
/// протокола X-Plane, не наш выбор.
const NAME_FIELD_LEN: usize = 400;
/// Полная длина пакета запроса: 5 (магия) + 4 (freq) + 4 (index) + 400 (имя).
const REQUEST_LEN: usize = 5 + 4 + 4 + NAME_FIELD_LEN;

/// Собирает 413-байтный запрос подписки/отписки протокола RREF.
///
/// `freq` — желаемая частота отправки значения в Гц, `0` отписывает от
/// dataref. `index` — присваиваемый клиентом идентификатор подписки; сим
/// вернёт его же в ответных пакетах, поэтому здесь всегда передаём
/// `DrIdx`-дискриминант, чтобы ответ сразу указывал на нужный слот в
/// массиве значений (см. `datarefs.rs`).
pub(crate) fn encode_request(freq: u32, index: usize, dataref: &str) -> [u8; REQUEST_LEN] {
    let mut buf = [0u8; REQUEST_LEN];
    buf[0..5].copy_from_slice(REQUEST_MAGIC);
    buf[5..9].copy_from_slice(&(freq as i32).to_le_bytes());
    buf[9..13].copy_from_slice(&(index as i32).to_le_bytes());

    let name = dataref.as_bytes();
    // Оставляем поле нулевым, если имя не влезло (не должно случиться на
    // наших именах — они на порядок короче 400 байт), но не паникуем.
    let n = name.len().min(NAME_FIELD_LEN);
    buf[13..13 + n].copy_from_slice(&name[..n]);

    buf
}

/// Разбирает один UDP-пакет ответа RREF (заголовок + N пар `{index:i32,
/// value:f32}` по 8 байт) и раскладывает значения по `values`/`seen`.
/// Возвращает число фактически обновлённых значений.
///
/// Проверяем только первые 4 байта заголовка (`"RREF"`) — пятый байт в
/// разных версиях X-Plane бывает и `\0`, и `,` (запятая), это не опечатка,
/// а расхождение самого протокола между версиями сима.
pub(crate) fn decode_packet(buf: &[u8], values: &mut [f32], seen: &mut [bool]) -> usize {
    if buf.len() < 5 || &buf[0..4] != b"RREF" {
        return 0;
    }

    let mut updated = 0;
    // Хвост короче 8 байт (неполная пара) просто отбрасываем — целых пар
    // в нём больше нет, а падать из-за обрезанного пакета незачем.
    let mut rest = &buf[5..];
    while rest.len() >= 8 {
        let index = i32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
        let value = f32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]);
        rest = &rest[8..];

        // Индекс от сима может ссылаться на прошлую (уже отменённую)
        // подписку — просто пропускаем, не паникуем на out-of-bounds.
        if index >= 0
            && let Some(slot) = values.get_mut(index as usize)
        {
            *slot = value;
            seen[index as usize] = true;
            updated += 1;
        }
    }

    updated
}

/// UDP-клиент протокола RREF.
pub struct RrefClient {
    sock: UdpSocket,
}

impl RrefClient {
    /// Один сокет и на отправку запросов подписки, и на приём значений: сим
    /// отвечает на адрес отправителя запроса, поэтому если слать с одного
    /// сокета, а слушать другим — ответы просто не придут.
    pub fn connect() -> std::io::Result<Self> {
        let sock = UdpSocket::bind("0.0.0.0:0")?;
        sock.connect("127.0.0.1:49000")?;
        // Короткий таймаут: drain() вызывается из цикла воркера с
        // собственным тактом, блокироваться на recv надолго нельзя.
        sock.set_read_timeout(Some(Duration::from_millis(100)))?;
        Ok(Self { sock })
    }

    /// Подписывается на все datarefs из `DrIdx::DEFS` с их штатной частотой.
    pub fn subscribe_all(&self) -> std::io::Result<()> {
        self.send_all(true)
    }

    /// Отписывается от всех datarefs (freq = 0) — вызывать при остановке
    /// воркера/потере фокуса на X-Plane, чтобы не засорять эфир после
    /// нашего ухода.
    pub fn unsubscribe_all(&self) -> std::io::Result<()> {
        self.send_all(false)
    }

    fn send_all(&self, subscribe: bool) -> std::io::Result<()> {
        for (i, &(dataref, freq)) in DrIdx::DEFS.iter().enumerate() {
            let req_freq = if subscribe { freq } else { 0 };
            let packet = encode_request(req_freq, i, dataref);
            self.sock.send(&packet)?;

            // Пауза каждые ~10 пакетов, чтобы не забить приёмный UDP-буфер
            // X-Plane сотней запросов подряд одной пачкой — сим сам их
            // разгребёт, но сплошной залп без пауз на практике роняет часть
            // пакетов на некоторых сетевых стеках/машинах.
            if i % 10 == 9 {
                thread::sleep(Duration::from_micros(200));
            }
        }
        Ok(())
    }

    /// Вычитывает все накопившиеся в сокете пакеты, пишет значения в
    /// `values`/`seen`. Возвращает суммарное число обновлённых значений (0,
    /// если новых данных не было). Не блокируется дольше таймаута сокета.
    pub fn drain(&self, values: &mut [f32], seen: &mut [bool]) -> usize {
        // 8 КиБ с запасом перекрывают самый плотный пакет, который может
        // прислать X-Plane при подписке на весь наш каталог (COUNT * 8 байт
        // на пару, обычно сим и так дробит их на несколько датаграмм).
        let mut buf = [0u8; 8192];
        let mut total = 0;

        loop {
            match self.sock.recv(&mut buf) {
                Ok(n) => total += decode_packet(&buf[..n], values, seen),
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => break,
                Err(_) => break,
            }
        }

        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_request_has_expected_layout() {
        let buf = encode_request(20, 7, "sim/flightmodel/position/indicated_airspeed");

        assert_eq!(buf.len(), 413);
        assert_eq!(&buf[0..5], b"RREF\0");

        let freq = i32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]);
        let index = i32::from_le_bytes([buf[9], buf[10], buf[11], buf[12]]);
        assert_eq!(freq, 20);
        assert_eq!(index, 7);

        let name_field = &buf[13..413];
        assert!(name_field.starts_with(b"sim/flightmodel/position/indicated_airspeed"));
        // Остаток поля добит нулями.
        let name_len = "sim/flightmodel/position/indicated_airspeed".len();
        assert!(name_field[name_len..].iter().all(|&b| b == 0));
    }

    #[test]
    fn unsubscribe_is_same_packet_with_zero_freq() {
        let buf = encode_request(0, 3, "sim/time/paused");
        let freq = i32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]);
        assert_eq!(freq, 0);
    }

    fn pair_bytes(index: i32, value: f32) -> [u8; 8] {
        let mut out = [0u8; 8];
        out[0..4].copy_from_slice(&index.to_le_bytes());
        out[4..8].copy_from_slice(&value.to_le_bytes());
        out
    }

    #[test]
    fn decode_packet_places_values_in_expected_slots() {
        let mut packet = Vec::new();
        packet.extend_from_slice(b"RREF\0");
        packet.extend_from_slice(&pair_bytes(0, 123.5));
        packet.extend_from_slice(&pair_bytes(2, -4.0));
        packet.extend_from_slice(&pair_bytes(5, 1.0));

        let mut values = [0.0f32; 6];
        let mut seen = [false; 6];
        let updated = decode_packet(&packet, &mut values, &mut seen);

        assert_eq!(updated, 3);
        assert_eq!(values[0], 123.5);
        assert_eq!(values[2], -4.0);
        assert_eq!(values[5], 1.0);
        assert!(seen[0] && seen[2] && seen[5]);
        assert!(!seen[1] && !seen[3] && !seen[4]);
    }

    #[test]
    fn decode_packet_accepts_both_header_variants() {
        let mut values = [0.0f32; 1];
        let mut seen = [false; 1];

        let mut with_nul = Vec::new();
        with_nul.extend_from_slice(b"RREF\0");
        with_nul.extend_from_slice(&pair_bytes(0, 1.0));
        assert_eq!(decode_packet(&with_nul, &mut values, &mut seen), 1);

        values = [0.0f32; 1];
        seen = [false; 1];
        let mut with_comma = Vec::new();
        with_comma.extend_from_slice(b"RREF,");
        with_comma.extend_from_slice(&pair_bytes(0, 2.0));
        assert_eq!(decode_packet(&with_comma, &mut values, &mut seen), 1);
        assert_eq!(values[0], 2.0);
    }

    #[test]
    fn decode_packet_rejects_garbage_header() {
        let mut values = [9.0f32; 2];
        let mut seen = [false; 2];
        let garbage = b"XXXX\0\x00\x00\x00\x00\x00\x00\x00\x00";

        let updated = decode_packet(garbage, &mut values, &mut seen);

        assert_eq!(updated, 0);
        assert_eq!(values, [9.0, 9.0]);
        assert_eq!(seen, [false, false]);
    }

    #[test]
    fn decode_packet_handles_short_and_truncated_packets_without_panic() {
        let mut values = [0.0f32; 4];
        let mut seen = [false; 4];

        // Короче заголовка.
        assert_eq!(decode_packet(b"RRE", &mut values, &mut seen), 0);

        // Заголовок есть, но пары данных обрезаны (не кратно 8 байтам).
        let mut truncated = Vec::new();
        truncated.extend_from_slice(b"RREF\0");
        truncated.extend_from_slice(&pair_bytes(1, 5.0));
        truncated.extend_from_slice(&[0xAB, 0xCD, 0xEF]); // хвост, 3 байта
        let updated = decode_packet(&truncated, &mut values, &mut seen);
        assert_eq!(updated, 1);
        assert_eq!(values[1], 5.0);
    }

    #[test]
    fn decode_packet_skips_out_of_range_index() {
        let mut values = [0.0f32; 2];
        let mut seen = [false; 2];

        let mut packet = Vec::new();
        packet.extend_from_slice(b"RREF\0");
        packet.extend_from_slice(&pair_bytes(99, 1.0)); // вне диапазона
        packet.extend_from_slice(&pair_bytes(1, 42.0)); // в диапазоне

        let updated = decode_packet(&packet, &mut values, &mut seen);

        assert_eq!(updated, 1);
        assert_eq!(values[1], 42.0);
        assert!(seen[1]);
        assert!(!seen[0]);
    }
}
