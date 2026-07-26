use std::thread;
use std::time::Duration;

fn main() {
    // Наша функция из скрипта
    let calc_transit = |pos: f64, prev_pos: f64, time: f64| -> f64 {
        if pos <= 0.0 || pos >= 50.0 || (pos - prev_pos).abs() < 0.001 {
            return 0.0;
        }

        let is_extending = pos > prev_pos;
        let p = if is_extending {
            pos / 50.0
        } else {
            1.0 - (pos / 50.0)
        };

        let mut effect = 0.0;

        // 1. Механические удары (Clunks)
        if p < 0.05 {
            effect += 12.0 * (1.0 - (p / 0.05)).powi(2);
        } else if p > 0.95 {
            effect += 16.0 * ((p - 0.95) / 0.05).powi(3);
        }

        // 2. Аэродинамический гул (Wind Buffet)
        let buffet_env = (std::f64::consts::PI * p).sin();
        let buffet_wave = (2.0 * std::f64::consts::PI * 8.0 * time).sin() * 0.5 + 0.5;
        effect += buffet_env * buffet_wave * 4.0;

        // 3. Звук гидравлики (Hydraulic Whine)
        let hydraulic_wave = (2.0 * std::f64::consts::PI * 35.0 * time).sin() * 0.5 + 0.5;
        effect += 1.0 + hydraulic_wave;

        effect
    };

    println!("Симуляция УБОРКИ шасси (50.0 -> 0.0) за 5 секунд...\n");

    let mut time = 0.0;
    let mut pos = 50.0;
    let mut prev_pos = 50.0;
    
    // Симулируем 60 кадров в секунду (dt = 0.016)
    let dt = 0.016; 
    let speed = 50.0 / (5.0 / dt); // Шасси убирается ровно 5 секунд

    while pos > 0.0 {
        pos -= speed;
        if pos < 0.0 {
            pos = 0.0;
        }

        let effect = calc_transit(pos, prev_pos, time);
        
        // Визуализация графика (1 символ = 1 единица мощности)
        let bars = "█".repeat(effect.round() as usize);
        
        println!("Time: {:4.2}s | Pos: {:5.2} | Effect: {:5.2} | {}", time, pos, effect, bars);

        prev_pos = pos;
        time += dt;

        // Пауза, чтобы график в терминале рисовался в реальном времени
        thread::sleep(Duration::from_millis(16));
    }

    println!("\nУборка завершена (шасси на замках).");
}