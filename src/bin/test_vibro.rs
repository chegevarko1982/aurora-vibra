use hidapi::HidApi;
use std::thread;
use std::time::Duration;

fn main() {
    println!("Инициализация HID API...");
    let api = HidApi::new().expect("Не удалось инициализировать HID API");

    println!("Ищем джойстик Winwing...");
    let device = api.open(0x4098, 0xB920).expect("Джойстик не найден! Убедись, что закрыл SimAppPro.");
    println!("Джойстик успешно подключен!\n");

    // Наш базовый массив команд (14 байт)
    let mut buf = [0u8; 14];
    buf[0] = 0x02; // Report ID
    buf[1] = 0x10;
    buf[2] = 0xb9;
    buf[3] = 0x00;
    buf[4] = 0x00;
    buf[5] = 0x03;
    buf[6] = 0x49;

    // Высчитываем задержку для плавного нарастания (10 сек / 256 шагов ≈ 39 мс)
    let step_delay = Duration::from_millis(39);

    // --- ТЕСТ 1: ЛЕВЫЙ МОТОР (Нарастание) ---
    println!("--- ЗАПУСК ЛЕВОГО МОТОРА (Плавное нарастание 10 сек) ---");
    buf[7] = 0x0e; // Адрес левого мотора
    
    for intensity in 0..=255 {
        buf[8] = intensity; // Устанавливаем текущую мощность
        device.write(&buf).expect("Ошибка отправки данных");
        
        // Чтобы в консоли было видно процесс (выводим каждое 10-е значение, чтобы не спамить)
        if intensity % 10 == 0 {
            println!("Левый мотор: мощность {}/255", intensity);
        }
        
        thread::sleep(step_delay);
    }

    // Выключаем левый мотор
    println!("Выключаем левый мотор...");
    buf[8] = 0x00; 
    device.write(&buf).expect("Ошибка выключения");

    // Пауза перед вторым мотором
    println!("Пауза 2 секунды...\n");
    thread::sleep(Duration::from_secs(2));

    // --- ТЕСТ 2: ПРАВЫЙ МОТОР (Нарастание) ---
    println!("--- ЗАПУСК ПРАВОГО МОТОРА (Плавное нарастание 10 сек) ---");
    buf[7] = 0x10; // Адрес правого мотора
    
    for intensity in 0..=255 {
        buf[8] = intensity; // Устанавливаем текущую мощность
        device.write(&buf).expect("Ошибка отправки данных");
        
        if intensity % 10 == 0 {
            println!("Правый мотор: мощность {}/255", intensity);
        }
        
        thread::sleep(step_delay);
    }

    // Выключаем правый мотор
    println!("Выключаем правый мотор...");
    buf[8] = 0x00; 
    device.write(&buf).expect("Ошибка выключения");

    println!("\nТест успешно завершен! Эффект плавного разгона проверен.");
}