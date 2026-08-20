//! Редактор пользовательских эффектов вибрации ("Мои эффекты") — подмодуль
//! `src/ui.rs` (см. `mod effects_editor;` там же). Вся отрисовка конструктора
//! вынесена сюда отдельно от остального интерфейса: `ui.rs` и так превысил
//! 3000 строк, а этот экран — самостоятельный "мастер" из 4 шагов со своими
//! графиками, раздувать им общий файл незачем.
//!
//! Дизайн подчинён одному требованию заказчика: пользователь должен собрать
//! эффект мышкой, ни разу не открыв документацию. Отсюда — постоянно видимые
//! живые графики на каждом шаге (что происходит с сигналом прямо сейчас) и
//! человеческие подписи вместо голых полей ввода. Все графики нарисованы
//! вручную через `ui.painter()` — `egui_plot` в проекте не подключён и
//! подключать его нельзя (см. задачу), да он тут и не нужен: графиков всего
//! три вида, все — простые линии/столбики.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use egui::{Color32, Rect, RichText, Sense, Vec2};

use crate::custom_fx::engine;
use crate::custom_fx::model::{
    CurvePoint, CustomEffect, GameMask, LvarSpec, MAX_SHAPE_FREQ_HZ, MixMode, ResponseCurve, Shape,
    Trigger, new_effect,
};
use crate::custom_fx::player::{self, Session, SessionFrame};
use crate::custom_fx::sources::{self, SOURCES, SourceDef, SourceId, SourceKind, TelemetryFrame};
use crate::custom_fx::store::{self, CustomFxShared};
use crate::file_dialog;
use crate::game_state::PreviewLock;
use crate::i18n::{Lang, Strings};
use crate::log::LogBuffer;
use crate::types::{ActiveGame, HidCmd};

use super::UiState;
use super::palette;

/// Ширина левой колонки со списком эффектов. Кнопки под списком рисуются
/// каждая на своей строке (а не в ряд) именно из-за этой узкой колонки —
/// русские подписи ("Дублировать", "Удалить") в ряд туда не влезли бы.
const EFFECT_LIST_WIDTH: f32 = 230.0;
const EFFECT_LIST_SCROLL_HEIGHT: f32 = 220.0;

/// Окно живой истории для мини-графиков шагов 1 и 2 — секунд достаточно,
/// чтобы увидеть тренд (разгон/торможение), но не настолько много, чтобы
/// график превратился в нечитаемую простыню при высокочастотном сигнале.
const LIVE_HISTORY_SECONDS: f64 = 15.0;
const MINI_GRAPH_HEIGHT: f32 = 56.0;
const CURVE_CANVAS_HEIGHT: f32 = 150.0;
const OSCILLOSCOPE_HEIGHT: f32 = 70.0;
const OSCILLOSCOPE_WINDOW_S: f64 = 2.0;
const OSCILLOSCOPE_SAMPLES: usize = 240;

/// Высота графика прогона записанной сессии (Задача B) — заметно выше
/// мини-графиков шагов 1/2: это главная ценность фичи ("видно, где за
/// реальный вылет эффект срабатывал"), ей одной ужиматься до 56px незачем.
const SESSION_GRAPH_HEIGHT: f32 = 90.0;

/// Точка холста кривой отклика, за которую можно "схватиться" мышью —
/// область попадания заметно больше самого нарисованного кружка, иначе
/// попасть курсором в 4-пиксельную точку было бы мучением.
const CURVE_POINT_RADIUS: f32 = 4.5;
const CURVE_POINT_HIT_SIZE: f32 = 18.0;

/// Раз в столько сохраняем список эффектов на диск, если в нём есть
/// несохранённые правки — отдельной кнопки "Сохранить" в конструкторе
/// сознательно нет (см. задачу), пользователь не обязан о ней помнить.
const SAVE_INTERVAL: Duration = Duration::from_millis(1500);

/// Троттлинг отправки `HidCmd::SendIntensity` во время предпросмотра —
/// ровно тот же интервал, что `SEND_INTERVAL` в `hid/worker.rs` (константа
/// там приватная модулю, не импортируется, поэтому дублируем значение, а не
/// саму константу). Слать чаще бессмысленно: канал/устройство всё равно не
/// пропускают обновления быстрее этого интервала.
const PREVIEW_SEND_INTERVAL: Duration = Duration::from_millis(50);

/// Пауза между повторами событийного эффекта (`is_event_effect`) в зацикленном
/// ручном предпросмотре — см. `preview_playback_time_s`. Без паузы соседние
/// повторы одиночного удара сливались бы в одно долгое дребезжание; величина
/// не из спеки, ориентир — "рука успевает отпустить ощущение прошлого удара"
/// прежде, чем прилетит следующий.
const PREVIEW_EVENT_REPEAT_GAP_S: f64 = 0.6;

/// Самые ходовые единицы SimConnect для `AddToDataDefinition` (см. doc-
/// комментарий `LvarSpec::unit`) — предлагаются выпадающим списком в
/// `show_lvar_source_fields`, поверх них пользователь может вписать любую
/// свою строку для редких единиц. Значения не локализуются: это буквальные
/// идентификаторы, которые уйдут в SimConnect как есть, а не подписи для
/// человека.
const LVAR_UNIT_PRESETS: &[&str] = &[
    "Number", "Percent", "Bool", "Knots", "Feet", "Degrees", "RPM", "Seconds",
];

/// Единица по умолчанию для свежесозданной `LvarSpec` — см. задачу: "По
/// умолчанию Number — подавляющее большинство LVAR именно такие".
const DEFAULT_LVAR_UNIT: &str = "Number";

/// Состояние редактора между кадрами — живёт в `UiState::fx_editor`. Сами
/// эффекты сюда НЕ входят (они в `CustomFxShared`, общем с движком) — здесь
/// только то, что нужно исключительно этому экрану: что выбрано, что рисуем
/// на графике, когда в последний раз сохраняли.
pub struct EditorState {
    /// id эффекта, выбранного в списке слева. `None` — список пуст либо
    /// выбор ещё не определён (первый кадр после запуска).
    selected_id: Option<String>,
    /// Кольцевой буфер живого значения источника ТЕКУЩЕГО выбранного эффекта
    /// за последние `LIVE_HISTORY_SECONDS` — общий для мини-графиков шагов 1
    /// и 2 (обновляется один раз за кадр, см. `update_live_history`).
    live_history: VecDeque<(Instant, f32)>,
    /// Источник, для которого набран `live_history` — как только пользователь
    /// меняет Step 1 на другой источник (или переключает выбранный эффект),
    /// буфер обязан обнулиться, иначе на графике был бы хвост чужого сигнала.
    live_history_source: Option<SourceId>,
    /// Имя LVAR, для которого набран `live_history`, когда
    /// `live_history_source == Some(SourceId::Lvar)` — `SourceId` сам по себе
    /// не меняется при правке имени переменной (это фиксированный вариант
    /// enum, см. doc-комментарий `SourceId::Lvar`), поэтому смену ИМЕНИ
    /// приходится отслеживать отдельным полем, иначе график продолжал бы
    /// рисовать историю уже переименованной переменной.
    live_history_lvar_name: Option<String>,
    /// Есть ли несохранённая на диск правка.
    dirty: bool,
    last_save_at: Instant,

    /// Пробное значение слайдера предпросмотра (шаг "Предпросмотр"), в
    /// единицах ТЕКУЩЕГО источника — своё для каждого источника по тому же
    /// приёму, что `live_history`/`live_history_source` выше: иначе при
    /// смене источника слайдер держал бы число из чужих единиц/диапазона.
    test_value: f64,
    test_value_source: Option<SourceId>,
    /// id эффекта, под который `test_value` был выставлен в последний раз.
    /// Одного `test_value_source` мало: два эффекта на ОДНОМ источнике
    /// (особенно на `Lvar`) могут иметь кривые с совершенно разными
    /// диапазонами по X, и число, осмысленное для одного, для другого
    /// оказывается прижатым к нулю.
    test_value_effect: Option<String>,

    /// id эффекта, который СЕЙЧАС реально играет на железе — `None` значит
    /// "предпросмотр выключен", единый источник истины (вместо отдельных
    /// bool+id, которые могли бы разойтись). См. `stop_preview` ниже: это
    /// единственное место, где предпросмотр может начаться/закончиться.
    preview_effect_id: Option<String>,
    /// Момент нажатия «Играть» — от него считается `t` в `engine::preview_level`.
    preview_started_at: Instant,
    /// Когда последний раз реально ушёл `HidCmd::SendIntensity` — троттлинг
    /// до `PREVIEW_SEND_INTERVAL`.
    preview_last_sent: Instant,
    /// Последние отправленные три канала (0..255) — только для индикации
    /// рядом с кнопкой; без неё непонятно, что вообще происходит, когда
    /// моторы физически не подключены к машине разработчика.
    preview_last_levels: (u8, u8, u8),
    /// Откуда предпросмотр берёт сырое значение, пока `preview_effect_id`
    /// не `None` — ручной слайдер (`test_value`) или проигрывание записи
    /// (Задача B). `preview_effect_id` остаётся ЕДИНСТВЕННЫМ переключателем
    /// "предпросмотр идёт вообще" (см. `stop_preview`), это поле лишь
    /// уточняет источник ВНУТРИ уже идущего предпросмотра — тот же канал и
    /// тот же `PreviewLock` обслуживают оба режима, второй независимый
    /// механизм захвата заводить незачем (и запрещено задачей).
    preview_source: PreviewSource,

    /// Загруженная кнопкой `btn_fx_open_session` запись — `None`, пока
    /// пользователь ничего не открыл (тогда в UI видна `lbl_fx_session_none`).
    session: Option<Session>,
    /// Офлайн-прогон текущего эффекта по ВСЕЙ `session` — пересчитывается не
    /// каждый кадр GUI, а только когда реально поменялись эффект или сама
    /// запись (см. `SessionSeries` и её сравнение в `step_preview`):
    /// прогон по многотысячному .jsonl не бесплатен, гонять его 30 раз в
    /// секунду вхолостую незачем.
    session_series: Option<SessionSeries>,
    /// Позиция курсора воспроизведения записи, секунды от начала сессии
    /// (t первого кадра = 0). Общая и для реального проигрывания на
    /// железе, и для ручного скраба мышью по графику — см. `step_preview`.
    session_cursor_s: f64,
}

/// См. doc-комментарий `EditorState::preview_source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum PreviewSource {
    #[default]
    TestValue,
    Session,
}

/// Результат офлайн-прогона эффекта по записанной сессии (Задача B): для
/// каждого кадра `session.frames` — либо посчитанные три канала, либо
/// `None`, если `sources::read` не смог прочитать источник ИМЕННО из этого
/// кадра (запись сделана в другой игре, чем источник эффекта — см. правило
/// задачи "не подставляй нули как настоящие значения"). Индексы совпадают
/// 1:1 с `session.frames`, поэтому рисование — простой zip двух срезов.
struct SessionSeries {
    levels: Vec<Option<(u8, u8, u8)>>,
    /// Путь записи и снимок эффекта, для которых посчитан `levels` — сверяем
    /// оба на каждый кадр GUI, чтобы понять, не протух ли кэш.
    for_session_path: PathBuf,
    for_effect: CustomEffect,
    /// Задача 2: `true`, когда НИ ОДИН кадр записи не принадлежит игре
    /// источника эффекта — типичный симптом "запись сделана в другой игре",
    /// который иначе выглядит неотличимо от пустой/битой записи. Считается
    /// один раз вместе с `levels` в `recompute_session_series`, а не заново
    /// каждый кадр GUI (см. кэш `session_series`, `needs_recompute` в
    /// `step_session`).
    game_mismatch: bool,
}

impl Default for EditorState {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            selected_id: None,
            live_history: VecDeque::new(),
            live_history_source: None,
            live_history_lvar_name: None,
            dirty: false,
            last_save_at: now,
            test_value: 0.0,
            test_value_source: None,
            test_value_effect: None,
            preview_effect_id: None,
            preview_started_at: now,
            preview_last_sent: now,
            preview_last_levels: (0, 0, 0),
            preview_source: PreviewSource::default(),
            session: None,
            session_series: None,
            session_cursor_s: 0.0,
        }
    }
}

impl EditorState {
    /// Единственный путь остановки предпросмотра — вызывается из ВСЕХ мест,
    /// где он обязан прерваться: явная кнопка "Стоп", выбор другого эффекта,
    /// удаление проигрываемого эффекта, переключение режима движка (все три —
    /// здесь же, в этом модуле) и уход с секции "Мои эффекты" (страховка в
    /// самом начале кадра в `ui.rs::UiState::ui`, единственном месте, которое
    /// гарантированно выполняется каждый кадр независимо от того, что сейчас
    /// отрисовано). Идемпотентен — no-op, если предпросмотр и так не играет.
    ///
    /// Кадр нулей уходит ПЕРЕД `release()`: воркер отпущенного канала
    /// подхватит его не раньше своего следующего тика, и если не погасить
    /// моторы явно здесь, до этого тика они простоят на последнем
    /// предпросмотренном значении, а не на нуле.
    pub fn stop_preview(&mut self, tx_hid: &Sender<HidCmd>, preview: &PreviewLock) {
        if self.preview_effect_id.is_none() {
            return;
        }
        let _ = tx_hid.send(HidCmd::SendIntensity {
            joystick: 0,
            throttle_left: 0,
            throttle_right: 0,
        });
        preview.release();
        self.preview_effect_id = None;
        self.preview_last_levels = (0, 0, 0);
    }
}

/// Всё, что редактору нужно из внешнего мира за один кадр — собирается
/// вызывающим кодом (`src/ui.rs`) перед вызовом `show`. Поля-ссылки живут не
/// дольше одного кадра отрисовки.
pub struct EditorCtx<'a> {
    pub effects: &'a CustomFxShared,
    pub active_ids: &'a [String],
    /// Текущий тик телеметрии активной игры, уже приведённый к общему виду —
    /// `None`, если игра не обнаружена или телеметрии ещё не было ни разу.
    pub live: Option<TelemetryFrame>,
    pub active_game: ActiveGame,
    pub t: &'a Strings,
    pub lang: Lang,
    pub logs: &'a LogBuffer,
    /// Канал HID-команд и замок предпросмотра — нужны шагу "Предпросмотр"
    /// (`step_preview`), чтобы слать `SendIntensity` и захватывать/отпускать
    /// `PreviewLock`. См. doc-комментарий `game_state::PreviewLock`.
    pub tx_hid: &'a Sender<HidCmd>,
    pub preview: &'a PreviewLock,
}

/// Точка входа модуля — вызывается из `src/ui.rs` при `Section::Effects`.
pub fn show(ui: &mut egui::Ui, st: &mut EditorState, cx: &mut EditorCtx) {
    let selected_before = st.selected_id.clone();

    // Один локальный клон на весь кадр: все правки (список слева и все 4
    // шага справа) копятся в этой переменной, а в общий `CustomFxShared`
    // уходят ОДНИМ вызовом `with_mut` в конце — так `with_mut` сравнивает
    // весь список целиком один раз (и поднимает rev только при реальном
    // изменении), вместо десятка мелких `with_mut` по каждому виджету.
    let mut effects = cx.effects.get();
    let effects_before = effects.clone();

    if !effects
        .iter()
        .any(|e| Some(e.id.as_str()) == st.selected_id.as_deref())
    {
        st.selected_id = effects.first().map(|e| e.id.clone());
    }

    // Одна строка вместо снесённого переключателя режимов: раздел обязан
    // с первого взгляда объяснять, что встроенный набор никуда не делся и
    // продолжает работать рядом (см. custom_fx::overrides). Что именно
    // вытесняет ВЫБРАННЫЙ источник — говорит шаг 1 по месту.
    ui.label(
        RichText::new(cx.t.msg_fx_effects_coexist)
            .italics()
            .color(palette::TEXT_SECONDARY),
    );
    ui.add_space(6.0);

    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(EFFECT_LIST_WIDTH);
            show_effect_list(ui, st, cx, &mut effects);
        });

        // Список слева уже мог поменять выбор или удалить эффект (New/
        // Duplicate/Delete/клик по строке) ВЫШЕ в этом же кадре — если играющий
        // предпросмотром эффект перестал быть и выбранным, и существующим,
        // глушим ДО отрисовки шага "Предпросмотр" ниже. Иначе тот шаг
        // отрисуется уже для НОВОГО выбора (is_playing == false для него) и
        // никогда не позовёт stop_preview — PreviewLock так и останется
        // захваченным старым, уже не отрисованным состоянием кнопки.
        if let Some(playing_id) = st.preview_effect_id.clone() {
            let still_current = st.selected_id.as_deref() == Some(playing_id.as_str())
                && effects.iter().any(|e| e.id == playing_id);
            if !still_current {
                st.stop_preview(cx.tx_hid, cx.preview);
            }
        }

        ui.add(egui::Separator::default().vertical());
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 4.0).max(220.0));
            let idx = st
                .selected_id
                .as_deref()
                .and_then(|id| effects.iter().position(|e| e.id == id));
            match idx {
                // A1: раньше правая колонка не скроллилась вообще — при
                // высоте окна <=1000px нижняя часть шага 4 (моторы) и вся
                // карточка предпросмотра были физически недоступны (не
                // доскроллить колесом мыши, живой баг с реального прогона).
                // Оборачиваем именно ЗДЕСЬ, а не глубже внутри
                // show_editor_steps, — так под скролл попадают все шаги
                // разом одной полосой, а не каждый в своей отдельной рамке.
                Some(i) => {
                    egui::ScrollArea::vertical()
                        .id_salt("fx_editor_steps_scroll")
                        .auto_shrink([false, false])
                        // Драг точки холста кривой (шаг 3, step_curve) не
                        // должен ОДНОВРЕМЕННО прокручивать эту ScrollArea.
                        // Дефолт egui (`DragScroll::OnTouch`) в теории не
                        // трогает обычную мышь, но на живом прогоне именно
                        // это и происходило (Windows транслирует ввод части
                        // дигитайзеров/трекпадов как touch-указатель) —
                        // поэтому глушим drag-to-scroll насовсем и явно, а
                        // не полагаемся на дефолт. Сам холст кривой уже
                        // реализует перетаскивание точки вручную через
                        // ui.interact(), полоса прокрутки и колесо мыши
                        // при этом продолжают работать как обычно.
                        .scroll_source(egui::scroll_area::ScrollSource {
                            drag: egui::scroll_area::DragScroll::Never,
                            ..Default::default()
                        })
                        .show(ui, |ui| {
                            show_editor_steps(ui, st, cx, &mut effects[i]);
                        });
                }
                None => {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(cx.t.empty_fx_hint).color(palette::TEXT_SECONDARY));
                    });
                }
            }
        });
    });

    if effects != effects_before {
        st.dirty = true;
    }
    cx.effects.with_mut(move |v| *v = effects);

    // Сохраняем на диск раз в SAVE_INTERVAL, а также сразу при смене
    // выбранного эффекта — иначе правки, сделанные секунду назад на другом
    // эффекте, могли бы не долежать до следующего тика таймера, если
    // пользователь тут же закрыл программу.
    let switched_selection = st.selected_id != selected_before;
    let due_by_timer = st.last_save_at.elapsed() >= SAVE_INTERVAL;
    if st.dirty && (due_by_timer || switched_selection) {
        if let Err(e) = store::save(&cx.effects.get()) {
            cx.logs
                .push(format!("Custom FX: failed to save effects: {e}"));
        }
        st.dirty = false;
        st.last_save_at = Instant::now();
    }
}

// ---------------------------------------------------------------------
// Задача 1 — заготовки нового эффекта
// ---------------------------------------------------------------------
//
// Пустой эффект по умолчанию (Trigger::Always + Shape::Constant + линейная
// кривая) гудит непрерывно и ничего не показывает — новичку не с чего
// начать. 4 заготовки ниже — сразу работающие эффекты, которые остаётся
// только подстроить под свой борт/предпочтения. Числа НЕ выдуманы — взяты
// из уже откалиброванных на живом железе значений встроенных эффектов, см.
// комментарий у каждой ветки `apply_preset`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EffectPreset {
    /// Короткий импульс на событие (касание ВПП, выпуск шасси).
    Impact,
    /// Текстурный гул, пока значение выше порога (превышение скорости, срыв потока).
    Hum,
    /// Мягкая плавная пульсация, пока значение выше порога — фоновое предупреждение.
    Pulsation,
    /// Сила прямо следует за значением источника — прежнее поведение "+ Новый эффект".
    Growing,
}

const EFFECT_PRESETS: [EffectPreset; 4] = [
    EffectPreset::Impact,
    EffectPreset::Hum,
    EffectPreset::Pulsation,
    EffectPreset::Growing,
];

fn preset_label(p: EffectPreset, t: &Strings) -> &'static str {
    match p {
        EffectPreset::Impact => t.preset_impact,
        EffectPreset::Hum => t.preset_hum,
        EffectPreset::Pulsation => t.preset_pulsation,
        EffectPreset::Growing => t.preset_growing,
    }
}

fn preset_hover(p: EffectPreset, t: &Strings) -> &'static str {
    match p {
        EffectPreset::Impact => t.hover_preset_impact,
        EffectPreset::Hum => t.hover_preset_hum,
        EffectPreset::Pulsation => t.hover_preset_pulsation,
        EffectPreset::Growing => t.hover_preset_growing,
    }
}

/// Настраивает СВЕЖЕсозданный эффект (уже с `source`/`name`/`id` от
/// `new_effect`) под одну из 4 заготовок — триггер, форма и кривая. `def` —
/// определение уже выставленного `effect.source` (диапазон/единицы).
fn apply_preset(effect: &mut CustomEffect, preset: EffectPreset, def: &SourceDef) {
    let (lo, hi) = def.default_range;
    // Тот же приём, что default_trigger_for_kind: минимальный шаг гистерезиса/
    // eps — доля диапазона источника, а не абсолютное число (иначе он был бы
    // либо ничтожным для скорости в км/ч, либо огромным для угла в градусах).
    let span = (hi - lo).abs().max(1.0);
    match preset {
        EffectPreset::Impact => {
            // Та же огибающая, что BUMP_MAIN в rumble.rs (касание стойки
            // шасси, подтверждено на живом железе): attack 0.10 с,
            // hold(severity=1) 0.20 с, decay 0.13..0.25 с (берём середину,
            // 0.19 с), decay_exp=2.
            effect.shape = Shape::OneShot {
                attack_ms: 100.0,
                hold_ms: 200.0,
                decay_ms: 190.0,
                decay_exp: 2,
            };
            effect.trigger = Trigger::Changed {
                eps: span * 0.02,
                // ДОЛЖНО перекрывать полную длительность огибающей выше
                // (0.10+0.20+0.19 = 0.49 с) — пока Changed не активен, движок
                // держит EMA (и тем самым итоговую интенсивность) в нуле
                // независимо от того, что рисует Shape::OneShot (см.
                // `engine::step`: raw_curve = 0.0, когда !is_active). Отпусти
                // триггер раньше конца decay — и хвост импульса обрежется.
                // 0.6 с — с запасом.
                hold_s: 0.6,
            };
            effect.curve = ResponseCurve::linear(lo, hi);
        }
        EffectPreset::Hum => {
            // weapon2_gun из WtConfig::default (types.rs) — "медленный ствол"
            // (текстура авиапушки), подтверждён на живом железе: несущая
            // 4.3 Гц уже безопасно проходит 20 Гц HID-канал без алиасинга
            // (см. комментарий там же), duty 40%, jitter 3%, attack 22 мс.
            // floor_pct = floor/peak = 100/255 в тех же единицах.
            let threshold = lo + span * 0.5;
            effect.shape = Shape::Pulse {
                freq_hz: 4.3,
                duty_pct: 40.0,
                jitter_pct: 3.0,
                floor_pct: 100.0 / 255.0 * 100.0,
                attack_ms: 22.0,
            };
            effect.trigger = Trigger::Above {
                value: threshold,
                hysteresis: span * 0.02,
            };
            // "Кривая нарастает от порога" — до порога эффект молчит
            // (Trigger::Above), а на самом пороге сразу начинает расти к
            // максимуму диапазона, а не стартует от 0% интенсивности.
            effect.curve = ResponseCurve::linear(threshold, hi);
        }
        EffectPreset::Pulsation => {
            // Та же логика триггера/кривой, что Hum, но Sine вместо
            // текстурированного Pulse — цель мягкость, а не текстура
            // конкретного оружия, поэтому под неё нет откалиброванного на
            // железе прототипа. Частота взята вдвое ниже потолка канала
            // (4.3 Гц у Hum) — умышленно медленнее и мягче на слух/ощупь.
            let threshold = lo + span * 0.5;
            effect.shape = Shape::Sine {
                freq_hz: 2.0,
                depth_pct: 100.0,
            };
            effect.trigger = Trigger::Above {
                value: threshold,
                hysteresis: span * 0.02,
            };
            effect.curve = ResponseCurve::linear(threshold, hi);
        }
        EffectPreset::Growing => {
            // Прежнее (до этой задачи) поведение "+ Новый эффект" — сила
            // прямо следует за значением источника, без триггера/формы.
            effect.shape = Shape::Constant;
            effect.trigger = Trigger::Always;
            effect.curve = ResponseCurve::linear(lo, hi);
        }
    }
}

/// Левая колонка: список эффектов + New/Duplicate/Delete/Import/Export.
fn show_effect_list(
    ui: &mut egui::Ui,
    st: &mut EditorState,
    cx: &EditorCtx,
    effects: &mut Vec<CustomEffect>,
) {
    // Задача 3: подсказка "эффектов пока нет" раньше дублировалась — тут И
    // на всю правую панель (см. `show`, ветка `None` пустого выбора). Правая
    // панель шире и заметнее (это основная рабочая область), поэтому там она
    // остаётся единственным местом показа; список слева на пустом наборе
    // эффектов просто ничего не рисует — ниже всё равно сразу видна кнопка
    // "+ Новый эффект".
    egui::ScrollArea::vertical()
        .id_salt("fx_effect_list_scroll")
        .max_height(EFFECT_LIST_SCROLL_HEIGHT)
        .show(ui, |ui| {
            for effect in effects.iter_mut() {
                show_effect_row(ui, st, cx, effect);
            }
        });

    ui.add_space(6.0);
    ui.separator();

    // Задача 1: вместо пустышки (Trigger::Always + Shape::Constant + линейная
    // кривая, которая гудит непрерывно и ничего не показывает новичку) кнопка
    // открывает меню из 4 готовых заготовок — см. `apply_preset` выше.
    // Источник по-прежнему подставляется по активной игре, заготовка только
    // доопределяет триггер/форму/кривую поверх него.
    ui.menu_button(cx.t.btn_fx_new, |ui| {
        for preset in EFFECT_PRESETS {
            if ui
                .button(preset_label(preset, cx.t))
                .on_hover_text(preset_hover(preset, cx.t))
                .clicked()
            {
                let source = default_source_for_game(cx.active_game);
                let def = sources::def(source);
                // Задача 4: раньше имя было "<источник> — <заготовка>"
                // ("Приборная скорость (IAS) (kn) — Удар") — в узкой левой
                // колонке (EFFECT_LIST_WIDTH) оно обрезалось прямо под
                // галочкой активности. Короткое имя заготовки читается и
                // само по себе; уникальность среди уже существующих
                // эффектов по-прежнему обеспечивает unique_effect_name
                // (второй "Удар" станет "Удар 2").
                let base = preset_label(preset, cx.t);
                let name = unique_effect_name(base, effects);
                let mut e = new_effect(name, source);
                apply_preset(&mut e, preset, def);
                // Булевы источники по смыслу требуют IsTrue независимо от
                // заготовки — см. doc-комментарий SourceKind::Boolean в
                // sources.rs. default_source_for_game сейчас всегда отдаёт
                // непрерывный источник, так что ветка сейчас недостижима, но
                // это дешёвая защита на случай, если это когда-нибудь
                // изменится.
                if def.kind == SourceKind::Boolean {
                    e.trigger = Trigger::IsTrue;
                }
                st.selected_id = Some(e.id.clone());
                effects.push(e);
                ui.close();
            }
        }
    });

    let has_selection = st.selected_id.is_some();
    if ui
        .add_enabled(has_selection, egui::Button::new(cx.t.btn_fx_duplicate))
        .clicked()
        && let Some(id) = st.selected_id.clone()
        && let Some(pos) = effects.iter().position(|e| e.id == id)
    {
        let mut dup = effects[pos].clone();
        // A4: та же проверка уникальности имени, что у "+ Новый эффект" —
        // без неё копия называлась бы точь-в-точь как оригинал.
        dup.name = unique_effect_name(&dup.name, effects);
        // Копия получает новый уникальный id — движок ключует состояние
        // (EMA/гистерезис/фазу Pulse) именно по id, общий id для оригинала и
        // копии перепутал бы их состояния (тот же приём, что в
        // store::import_from при коллизии id).
        dup.id = new_effect(dup.name.clone(), dup.source).id;
        st.selected_id = Some(dup.id.clone());
        effects.insert(pos + 1, dup);
    }

    if ui
        .add_enabled(has_selection, egui::Button::new(cx.t.btn_fx_delete))
        .clicked()
        && let Some(id) = st.selected_id.clone()
        && let Some(pos) = effects.iter().position(|e| e.id == id)
    {
        effects.remove(pos);
        st.selected_id = effects.first().map(|e| e.id.clone());
    }

    ui.add_space(4.0);
    ui.separator();

    if ui.button(cx.t.btn_fx_export).clicked()
        && let Some(path) = file_dialog::save_file(
            cx.t.dlg_fx_export_title,
            cx.t.filter_fx_file,
            "json",
            "aurora-vibra-effects.json",
            None,
        )
    {
        match store::export_to(&path, effects) {
            Ok(()) => cx.logs.push(format!(
                "Custom FX: exported {} effect(s) to {}",
                effects.len(),
                path.display()
            )),
            Err(e) => cx
                .logs
                .push(format!("Custom FX: failed to export effects: {e}")),
        }
    }

    if ui.button(cx.t.btn_fx_import).clicked()
        && let Some(path) =
            file_dialog::open_file(cx.t.dlg_fx_import_title, cx.t.filter_fx_file, "json", None)
    {
        match store::import_from(&path, effects) {
            Ok(imported) => {
                let n = imported.len();
                effects.extend(imported);
                cx.logs.push(format!(
                    "Custom FX: imported {n} effect(s) from {}",
                    path.display()
                ));
            }
            Err(e) => cx
                .logs
                .push(format!("Custom FX: failed to import effects: {e}")),
        }
    }
}

fn show_effect_row(
    ui: &mut egui::Ui,
    st: &mut EditorState,
    cx: &EditorCtx,
    effect: &mut CustomEffect,
) {
    let selected = st.selected_id.as_deref() == Some(effect.id.as_str());
    let active = cx.active_ids.iter().any(|id| id == &effect.id);

    let frame_resp = egui::Frame::new()
        .fill(if selected {
            palette::SELECTED_BG
        } else {
            Color32::TRANSPARENT
        })
        .corner_radius(4u8)
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let dot_color = if active {
                    Color32::WHITE
                } else {
                    palette::BORDER_ACTIVE
                };
                dot_indicator_pub(ui, dot_color, active, 8.0);

                let name_color = if selected {
                    palette::SELECTED_FG
                } else if effect.enabled {
                    palette::TEXT_PRIMARY
                } else {
                    palette::TEXT_DISABLED
                };
                ui.label(RichText::new(&effect.name).color(name_color));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.checkbox(&mut effect.enabled, "");
                });
            });
        });

    // Клик по всей строке выбирает эффект (не только по надписи) — сама
    // Frame по умолчанию не сенсит клики, поэтому отдельный interact поверх
    // её прямоугольника.
    let row_click = ui.interact(
        frame_resp.response.rect,
        ui.id().with(("fx_effect_row", &effect.id)),
        Sense::click(),
    );
    if row_click.clicked() {
        st.selected_id = Some(effect.id.clone());
    }
}

/// Тонкая обёртка над приватным `dot_indicator` из `src/ui.rs` — он там не
/// `pub`, но виден нам как дочернему модулю (правило приватности Rust:
/// приватный элемент виден в определяющем модуле и во ВСЕХ его потомках).
/// Обёртка нужна только для говорящего имени на вызове; ничего не добавляет.
fn dot_indicator_pub(ui: &mut egui::Ui, color: Color32, filled: bool, diameter: f32) {
    super::dot_indicator(ui, color, filled, diameter);
}

fn show_editor_steps(
    ui: &mut egui::Ui,
    st: &mut EditorState,
    cx: &EditorCtx,
    effect: &mut CustomEffect,
) {
    update_live_history(st, cx, effect);

    step_source(ui, st, cx, effect);
    ui.add_space(6.0);
    step_trigger(ui, st, cx, effect);
    ui.add_space(6.0);
    step_curve(ui, cx, effect);
    ui.add_space(6.0);
    step_shape(ui, cx, effect);
    ui.add_space(6.0);
    show_warnings(ui, cx, effect);
    ui.add_space(6.0);
    step_preview(ui, st, cx, effect);
}

/// Единая точка чтения "сырого" значения источника ТЕКУЩЕГО эффекта для
/// живых виджетов UI (значение/мини-график/холст кривой/осциллограф) —
/// обычные источники идут через `sources::read`, а `SourceId::Lvar` — через
/// `sources::read_lvar` по имени из `effect.lvar` (само имя живёт на
/// эффекте, а не в таблице `SOURCES`, см. doc-комментарий `sources::read_lvar`
/// про то, почему это отдельная функция). Пустое/отсутствующее имя
/// переменной — `None`, как и обычное "нет сигнала" от прочих источников.
fn read_effect_value(effect: &CustomEffect, frame: &TelemetryFrame) -> Option<f64> {
    if effect.source == SourceId::Lvar {
        let name = effect.lvar.as_ref().map(|l| l.name.as_str()).unwrap_or("");
        if name.trim().is_empty() {
            return None;
        }
        sources::read_lvar(name, frame)
    } else {
        sources::read(effect.source, frame)
    }
}

/// Пополняет кольцевой буфер живой истории источника ТЕКУЩЕГО эффекта.
/// Сброс буфера при смене источника — иначе первые ~15 секунд после
/// переключения на график лез бы хвост значений другого источника, в других
/// единицах и диапазоне. Для `SourceId::Lvar` дополнительно сбрасывается при
/// смене ИМЕНИ переменной (см. doc-комментарий `EditorState::live_history_lvar_name`) —
/// сам `SourceId` при этом не меняется.
fn update_live_history(st: &mut EditorState, cx: &EditorCtx, effect: &CustomEffect) {
    let lvar_name = effect.lvar.as_ref().map(|l| l.name.clone());
    if st.live_history_source != Some(effect.source) || st.live_history_lvar_name != lvar_name {
        st.live_history.clear();
        st.live_history_source = Some(effect.source);
        st.live_history_lvar_name = lvar_name;
    }
    let Some(raw) = cx.live.as_ref().and_then(|f| read_effect_value(effect, f)) else {
        return;
    };
    let now = Instant::now();
    st.live_history.push_back((now, raw as f32));
    while let Some((t0, _)) = st.live_history.front() {
        if now.saturating_duration_since(*t0).as_secs_f64() > LIVE_HISTORY_SECONDS {
            st.live_history.pop_front();
        } else {
            break;
        }
    }
}

// ---------------------------------------------------------------------
// Шаг 1 — источник
// ---------------------------------------------------------------------

fn step_source(ui: &mut egui::Ui, st: &EditorState, cx: &EditorCtx, effect: &mut CustomEffect) {
    UiState::effect_card(ui, true, false, |ui| {
        ui.label(RichText::new(cx.t.step_fx_source).strong().size(15.0));
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label(cx.t.lbl_fx_name);
            ui.text_edit_singleline(&mut effect.name);
        });

        let mut new_source = None;
        let cur_def = sources::def(effect.source);
        egui::ComboBox::from_id_salt("fx_source_select")
            .selected_text(source_label(cur_def, cx.lang))
            .width(260.0)
            .show_ui(ui, |ui| {
                let mut prev_group: Option<SourceGroup> = None;
                for s in SOURCES.iter() {
                    let group = source_group(s.games);
                    if Some(group) != prev_group {
                        if prev_group.is_some() {
                            ui.separator();
                        }
                        // Без .small(): заголовок группы обязан читаться так же
                        // легко, как сами пункты — мелкий приглушённый текст в
                        // этом проекте уже один раз оказался нечитаемым (см.
                        // комментарии к палитре в ui.rs). От пунктов его
                        // отличают жирность, отступ и разделитель выше, а не
                        // размер.
                        ui.label(
                            RichText::new(source_group_header(group, cx.t))
                                .strong()
                                .color(palette::TEXT_SECONDARY),
                        );
                        prev_group = Some(group);
                    }
                    if ui
                        .selectable_label(s.id == effect.source, source_label(s, cx.lang))
                        .clicked()
                    {
                        new_source = Some(s.id);
                    }
                }
            });
        if let Some(src) = new_source {
            effect.source = src;
        }

        // Задача 1а: показываем СВЯЗЬ источник -> встроенный эффект ещё до
        // того, как пользователь включит и сохранит этот эффект (см.
        // doc-комментарий `super::static_primary_builtin_for`).
        match super::static_primary_builtin_for(effect.source) {
            Some(builtin) => {
                ui.label(
                    RichText::new(format!(
                        "{} {}",
                        cx.t.lbl_fx_overrides_builtin,
                        super::builtin_effect_display_name(builtin, cx.t)
                    ))
                    .color(palette::STATUS_ATTENTION)
                    .italics(),
                );
            }
            None => {
                ui.label(
                    RichText::new(cx.t.lbl_fx_overrides_builtin_none)
                        .color(palette::TEXT_SECONDARY)
                        .italics(),
                );
            }
        }

        if effect.source == SourceId::Lvar {
            show_lvar_source_fields(ui, cx, effect);
        }

        let def = sources::def(effect.source);
        let raw = cx.live.as_ref().and_then(|f| read_effect_value(effect, f));
        ui.horizontal(|ui| {
            ui.label(cx.t.lbl_fx_live_value);
            match raw {
                Some(v) => {
                    let text = match def.kind {
                        SourceKind::Boolean => {
                            if v > 0.5 { cx.t.val_on } else { cx.t.val_off }.to_string()
                        }
                        SourceKind::Continuous if def.unit.is_empty() => format!("{v:.1}"),
                        SourceKind::Continuous => format!("{v:.1} {}", def.unit),
                    };
                    ui.label(RichText::new(text).strong());
                }
                None => {
                    ui.label(RichText::new(cx.t.lbl_fx_no_signal).color(palette::TEXT_DISABLED));
                }
            }
        });
        // A2: раньше предупреждение показывалось и когда игры вообще нет
        // (ActiveGame::None) — а тогда "источник не для этой игры" неверно
        // по смыслу: игры нет вообще, а не "не той". Показываем ТОЛЬКО
        // когда игра реально активна и источник для неё не подходит.
        if cx.active_game != ActiveGame::None && !def.games.allows(cx.active_game) {
            ui.colored_label(palette::STATUS_WARN, cx.t.warn_fx_wrong_game);
        }

        draw_mini_graph(
            ui,
            &st.live_history,
            def.default_range,
            MINI_GRAPH_HEIGHT,
            TriggerOverlay::None,
            cx.t.lbl_fx_no_signal,
        );

        // Задача 3 (custom LVAR source), пункт 6: LVAR физически существует
        // только в MSFS (см. `MSFS_ONLY` в sources.rs) — маска игр
        // приводится к MSFS-only КАЖДЫЙ кадр, пока выбран этот источник, а
        // не только один раз при переключении: так эффект, загруженный уже
        // с "битой" маской (например, руками отредактированный экспортный
        // JSON), самоисправляется, а не остаётся в противоречивом
        // состоянии.
        let lvar_locked = effect.source == SourceId::Lvar;
        if lvar_locked {
            coerce_lvar_game_mask(&mut effect.games);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(cx.t.lbl_fx_games);
            ui.add_enabled(
                !lvar_locked,
                egui::Checkbox::new(&mut effect.games.msfs, cx.t.hover_game_msfs),
            );
            ui.add_enabled(
                !lvar_locked,
                egui::Checkbox::new(&mut effect.games.xplane, cx.t.hover_game_xp),
            );
            ui.add_enabled(
                !lvar_locked,
                egui::Checkbox::new(&mut effect.games.wt, cx.t.hover_game_wt),
            );
        });
        if lvar_locked {
            ui.label(
                RichText::new(cx.t.msg_fx_lvar_msfs_only)
                    .italics()
                    .color(palette::TEXT_SECONDARY),
            );
        }

        ui.horizontal(|ui| {
            ui.label(cx.t.lbl_fx_aircraft)
                .on_hover_text(cx.t.hover_fx_aircraft);
            let mut aircraft = effect.aircraft_match.clone().unwrap_or_default();
            if ui
                .text_edit_singleline(&mut aircraft)
                .on_hover_text(cx.t.hover_fx_aircraft)
                .changed()
            {
                effect.aircraft_match = if aircraft.trim().is_empty() {
                    None
                } else {
                    Some(aircraft)
                };
            }
        });
    });
}

/// Дополнительные поля шага 1 для `SourceId::Lvar` (задача 3, custom LVAR
/// source): имя переменной + подсказка, где его искать в симуляторе (пункт
/// 1 задачи), подсказка про типичный префикс `L:` (пункт 5), и единица
/// измерения для SimConnect (пункт 2). Живое значение/мини-график ниже по
/// `step_source` уже работают от `read_effect_value` и подхватывают эту же
/// `effect.lvar.name` сами — здесь их трогать не нужно, это и есть пункт 3
/// задачи, "самое важное".
///
/// Создаёт `effect.lvar` при первом вызове (пока источник только что
/// переключён на `Lvar` и поле ещё `None`) — до этого момента эффект
/// работает так, будто имя пустое (`read_effect_value` уже трактует пустое
/// имя как "нет сигнала", ничего не падает).
fn show_lvar_source_fields(ui: &mut egui::Ui, cx: &EditorCtx, effect: &mut CustomEffect) {
    let mut spec = effect.lvar.clone().unwrap_or_else(|| LvarSpec {
        name: String::new(),
        unit: DEFAULT_LVAR_UNIT.to_string(),
    });

    ui.horizontal(|ui| {
        ui.label(cx.t.lbl_fx_lvar_name);
        ui.text_edit_singleline(&mut spec.name);
    });
    // Пункт 1 задачи: подсказка, ГДЕ брать имена, всегда видна текстом (а не
    // только по наведению) — пользователь не обязан догадываться, что под
    // полем вообще есть tooltip.
    ui.label(
        RichText::new(cx.t.lbl_fx_lvar_hint)
            .small()
            .color(palette::TEXT_DISABLED),
    );
    if lvar_name_missing_prefix_hint(&spec.name) {
        ui.label(
            RichText::new(cx.t.lbl_fx_lvar_prefix_hint)
                .small()
                .color(palette::TEXT_DISABLED),
        );
    }

    let is_preset_unit = LVAR_UNIT_PRESETS.contains(&spec.unit.as_str());
    ui.horizontal(|ui| {
        ui.label(cx.t.lbl_fx_lvar_unit);
        egui::ComboBox::from_id_salt("fx_lvar_unit_select")
            .selected_text(if is_preset_unit {
                spec.unit.clone()
            } else {
                cx.t.opt_fx_lvar_unit_custom.to_string()
            })
            .show_ui(ui, |ui| {
                for u in LVAR_UNIT_PRESETS {
                    if ui.selectable_label(spec.unit == *u, *u).clicked() {
                        spec.unit = (*u).to_string();
                    }
                }
                // "Своя…" — выбор этого пункта только ОЧИЩАЕТ поле (открывая
                // текстовый ввод ниже), само по себе не является единицей.
                // Повторный клик, когда уже стоит своя единица, — no-op, не
                // затирает то, что пользователь уже вписал.
                if ui
                    .selectable_label(!is_preset_unit, cx.t.opt_fx_lvar_unit_custom)
                    .clicked()
                    && is_preset_unit
                {
                    spec.unit.clear();
                }
            });
    });
    if !is_preset_unit {
        ui.horizontal(|ui| {
            ui.label(cx.t.lbl_fx_lvar_unit_custom);
            ui.text_edit_singleline(&mut spec.unit);
        });
    }

    effect.lvar = Some(spec);
}

/// Пункт 6 задачи 3: приводит маску игр к MSFS-only на месте. Чистая
/// функция над `GameMask` — вынесена ради юнит-теста, сама по себе ничего
/// не решает (решение "приводить или нет" остаётся за вызывающим кодом,
/// см. `step_source`).
fn coerce_lvar_game_mask(games: &mut GameMask) {
    games.msfs = true;
    games.xplane = false;
    games.wt = false;
}

/// Пункт 5 задачи 3: `true`, когда в непустом имени переменной нет
/// двоеточия — типичный признак того, что пользователь не дописал
/// стандартный префикс локальной переменной (`L:...`). Пустое/пробельное
/// имя не считается — подсказка про префикс не нужна, пока нечего
/// префиксовать (там и так уже видно "нет сигнала").
fn lvar_name_missing_prefix_hint(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty() && !trimmed.contains(':')
}

// ---------------------------------------------------------------------
// Шаг 2 — триггер
// ---------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum TriggerKind {
    Always,
    Above,
    Below,
    Between,
    IsTrue,
    Changed,
}

const TRIGGER_KINDS: [TriggerKind; 6] = [
    TriggerKind::Always,
    TriggerKind::Above,
    TriggerKind::Below,
    TriggerKind::Between,
    TriggerKind::IsTrue,
    TriggerKind::Changed,
];

fn trigger_kind_of(t: &Trigger) -> TriggerKind {
    match t {
        Trigger::Always => TriggerKind::Always,
        Trigger::Above { .. } => TriggerKind::Above,
        Trigger::Below { .. } => TriggerKind::Below,
        Trigger::Between { .. } => TriggerKind::Between,
        Trigger::IsTrue => TriggerKind::IsTrue,
        Trigger::Changed { .. } => TriggerKind::Changed,
    }
}

fn trigger_kind_label(k: TriggerKind, t: &Strings) -> &'static str {
    match k {
        TriggerKind::Always => t.trigger_always,
        TriggerKind::Above => t.trigger_above,
        TriggerKind::Below => t.trigger_below,
        TriggerKind::Between => t.trigger_between,
        TriggerKind::IsTrue => t.trigger_is_true,
        TriggerKind::Changed => t.trigger_changed,
    }
}

fn default_trigger_for_kind(k: TriggerKind, def: &SourceDef) -> Trigger {
    let (lo, hi) = def.default_range;
    let mid = (lo + hi) / 2.0;
    let span = (hi - lo).abs().max(1.0);
    match k {
        TriggerKind::Always => Trigger::Always,
        TriggerKind::Above => Trigger::Above {
            value: mid,
            hysteresis: span * 0.02,
        },
        TriggerKind::Below => Trigger::Below {
            value: mid,
            hysteresis: span * 0.02,
        },
        // Весь диапазон источника по умолчанию — пользователь сузит сам,
        // а вырожденный "От == До" сразу после выбора триггера ничего бы не
        // показал на графике и выглядел бы как баг.
        TriggerKind::Between => Trigger::Between {
            lo: lo.min(hi),
            hi: lo.max(hi),
        },
        TriggerKind::IsTrue => Trigger::IsTrue,
        TriggerKind::Changed => Trigger::Changed {
            eps: span * 0.02,
            hold_s: 0.5,
        },
    }
}

enum TriggerOverlay {
    None,
    Line(f64),
    Band(f64, f64),
}

fn overlay_for_trigger(t: &Trigger) -> TriggerOverlay {
    match t {
        Trigger::Always | Trigger::IsTrue | Trigger::Changed { .. } => TriggerOverlay::None,
        Trigger::Above { value, .. } | Trigger::Below { value, .. } => TriggerOverlay::Line(*value),
        Trigger::Between { lo, hi } => TriggerOverlay::Band(*lo, *hi),
    }
}

fn step_trigger(ui: &mut egui::Ui, st: &EditorState, cx: &EditorCtx, effect: &mut CustomEffect) {
    UiState::effect_card(ui, true, false, |ui| {
        ui.label(RichText::new(cx.t.step_fx_when).strong().size(15.0));
        ui.add_space(4.0);

        let def = sources::def(effect.source);
        let cur_kind = trigger_kind_of(&effect.trigger);
        let mut new_kind = None;
        egui::ComboBox::from_id_salt("fx_trigger_select")
            .selected_text(trigger_kind_label(cur_kind, cx.t))
            .show_ui(ui, |ui| {
                for k in TRIGGER_KINDS {
                    let resp = ui.selectable_label(cur_kind == k, trigger_kind_label(k, cx.t));
                    let resp = if k == TriggerKind::Changed {
                        resp.on_hover_text(cx.t.hover_trigger_changed)
                    } else {
                        resp
                    };
                    if resp.clicked() {
                        new_kind = Some(k);
                    }
                }
            });
        if let Some(k) = new_kind {
            effect.trigger = default_trigger_for_kind(k, def);
        }

        match &mut effect.trigger {
            Trigger::Always | Trigger::IsTrue => {}
            Trigger::Above { value, hysteresis } | Trigger::Below { value, hysteresis } => {
                let (lo, hi) = def.default_range;
                let span = (hi - lo).abs().max(1.0);
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_threshold);
                    ui.add(egui::Slider::new(value, lo.min(hi)..=lo.max(hi)));
                });
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_hysteresis);
                    ui.add(egui::Slider::new(hysteresis, 0.0..=(span * 0.25)))
                        .on_hover_text(cx.t.hover_fx_hysteresis);
                });
            }
            Trigger::Between { lo: rlo, hi: rhi } => {
                let (lo, hi) = def.default_range;
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_range_lo);
                    ui.add(egui::Slider::new(rlo, lo.min(hi)..=lo.max(hi)));
                });
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_range_hi);
                    ui.add(egui::Slider::new(rhi, lo.min(hi)..=lo.max(hi)));
                });
                if *rhi < *rlo {
                    std::mem::swap(rlo, rhi);
                }
            }
            Trigger::Changed { eps, hold_s } => {
                let (lo, hi) = def.default_range;
                let span = (hi - lo).abs().max(1.0);
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_eps);
                    ui.add(
                        egui::DragValue::new(eps)
                            .speed(span * 0.01)
                            .range(0.0..=span),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_hold);
                    ui.add(egui::Slider::new(hold_s, 0.0..=5.0).suffix(" s"));
                });
            }
        }

        draw_mini_graph(
            ui,
            &st.live_history,
            def.default_range,
            MINI_GRAPH_HEIGHT,
            overlay_for_trigger(&effect.trigger),
            cx.t.lbl_fx_no_signal,
        );
    });
}

// ---------------------------------------------------------------------
// Шаг 3 — кривая отклика
// ---------------------------------------------------------------------

fn step_curve(ui: &mut egui::Ui, cx: &EditorCtx, effect: &mut CustomEffect) {
    UiState::effect_card(ui, true, false, |ui| {
        ui.label(RichText::new(cx.t.step_fx_curve).strong().size(15.0));
        ui.add_space(4.0);

        let def = sources::def(effect.source);
        let bounds = curve_x_bounds(&effect.curve, def.default_range);
        let y_bounds = (0.0, 1.0);

        let desired = Vec2::new(ui.available_width(), CURVE_CANVAS_HEIGHT);
        let (rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 4u8, palette::BG_APP);
        painter.rect_stroke(
            rect,
            4u8,
            egui::Stroke::new(1.0, palette::BORDER_DEFAULT),
            egui::StrokeKind::Inside,
        );

        // Сетка по Y (0/25/50/75/100%) — подписи только у крайних линий,
        // чтобы не захламлять маленький холст числами.
        for pct in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let y = value_to_px_y(pct, y_bounds, rect);
            painter.line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                egui::Stroke::new(1.0, palette::BORDER_DEFAULT.gamma_multiply(0.6)),
            );
        }
        let font = egui::FontId::proportional(10.0);
        painter.text(
            egui::pos2(rect.left() + 3.0, rect.top() + 2.0),
            egui::Align2::LEFT_TOP,
            "100%",
            font.clone(),
            palette::TEXT_DISABLED,
        );
        painter.text(
            egui::pos2(rect.left() + 3.0, rect.bottom() - 2.0),
            egui::Align2::LEFT_BOTTOM,
            "0%",
            font,
            palette::TEXT_DISABLED,
        );

        // Живое значение источника — вертикальная линия прямо на кривой,
        // чтобы было видно, куда сейчас "смотрит" эффект.
        if let Some(raw) = cx.live.as_ref().and_then(|f| read_effect_value(effect, f)) {
            let x = value_to_px_x(raw, bounds, rect).clamp(rect.left(), rect.right());
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(1.5, palette::ACCENT_LIVE),
            );
        }

        // Ломаная — по фактическому порядку точек в массиве (может быть
        // временно не отсортирован во время перетаскивания, см. ниже).
        let screen_points: Vec<egui::Pos2> = effect
            .curve
            .points
            .iter()
            .map(|p| {
                egui::pos2(
                    value_to_px_x(p.x, bounds, rect),
                    value_to_px_y(p.y, y_bounds, rect),
                )
            })
            .collect();
        for w in screen_points.windows(2) {
            painter.line_segment(
                [w[0], w[1]],
                egui::Stroke::new(2.0, palette::ACCENT_PRIMARY),
            );
        }

        let mut any_point_interacted = false;
        let mut remove_idx = None;
        let n = effect.curve.points.len();
        for (i, &center) in screen_points.iter().enumerate() {
            let point_rect = Rect::from_center_size(center, Vec2::splat(CURVE_POINT_HIT_SIZE));
            let id = response.id.with(("fx_curve_point", i));
            let presp = ui.interact(point_rect, id, Sense::click_and_drag());

            if presp.dragged()
                && let Some(pos) = presp.interact_pointer_pos()
            {
                let x = pos.x.clamp(rect.left(), rect.right());
                let y = pos.y.clamp(rect.top(), rect.bottom());
                effect.curve.points[i] = CurvePoint {
                    x: px_x_to_value(x, bounds, rect),
                    y: px_y_to_value(y, y_bounds, rect).clamp(0.0, 1.0),
                };
                any_point_interacted = true;
            }
            if presp.drag_stopped() {
                effect.curve.sorted();
            }
            if presp.secondary_clicked() && n > 2 {
                remove_idx = Some(i);
                any_point_interacted = true;
            }
            if presp.clicked() {
                any_point_interacted = true;
            }

            let point_color = if presp.dragged() || presp.hovered() {
                Color32::WHITE
            } else {
                palette::ACCENT_PRIMARY
            };
            painter.circle_filled(center, CURVE_POINT_RADIUS, point_color);
        }

        if let Some(i) = remove_idx {
            effect.curve.points.remove(i);
        }

        // Клик по свободному месту холста (мимо всех точек) добавляет новую
        // точку прямо на линии — то же самое "клик по линии", что описано в
        // подсказке lbl_fx_curve_hint.
        if response.clicked()
            && !any_point_interacted
            && let Some(pos) = response.interact_pointer_pos()
        {
            let x = px_x_to_value(pos.x, bounds, rect);
            let y = effect.curve.eval(x);
            effect.curve.points.push(CurvePoint { x, y });
            effect.curve.sorted();
        }

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{:.0}{}", bounds.0, unit_suffix(def)))
                    .small()
                    .color(palette::TEXT_DISABLED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{:.0}{}", bounds.1, unit_suffix(def)))
                        .small()
                        .color(palette::TEXT_DISABLED),
                );
            });
        });
        ui.label(
            RichText::new(cx.t.lbl_fx_curve_hint)
                .small()
                .color(palette::TEXT_DISABLED),
        );
    });
}

fn unit_suffix(def: &SourceDef) -> String {
    if def.unit.is_empty() {
        String::new()
    } else {
        format!(" {}", def.unit)
    }
}

// ---------------------------------------------------------------------
// Шаг 4 — форма и выход
// ---------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShapeKind {
    Constant,
    Pulse,
    OneShot,
    Sine,
    Sawtooth,
}

const SHAPE_KINDS: [ShapeKind; 5] = [
    ShapeKind::Constant,
    ShapeKind::Pulse,
    ShapeKind::OneShot,
    ShapeKind::Sine,
    ShapeKind::Sawtooth,
];

fn shape_kind_of(s: &Shape) -> ShapeKind {
    match s {
        Shape::Constant => ShapeKind::Constant,
        Shape::Pulse { .. } => ShapeKind::Pulse,
        Shape::OneShot { .. } => ShapeKind::OneShot,
        Shape::Sine { .. } => ShapeKind::Sine,
        Shape::Sawtooth { .. } => ShapeKind::Sawtooth,
    }
}

fn shape_kind_label(k: ShapeKind, t: &Strings) -> &'static str {
    match k {
        ShapeKind::Constant => t.shape_constant,
        ShapeKind::Pulse => t.shape_pulse,
        ShapeKind::OneShot => t.shape_oneshot,
        ShapeKind::Sine => t.shape_sine,
        ShapeKind::Sawtooth => t.shape_sawtooth,
    }
}

fn default_shape_for_kind(k: ShapeKind) -> Shape {
    match k {
        ShapeKind::Constant => Shape::Constant,
        ShapeKind::Pulse => Shape::Pulse {
            freq_hz: 4.0,
            duty_pct: 50.0,
            jitter_pct: 0.0,
            floor_pct: 0.0,
            attack_ms: 0.0,
        },
        ShapeKind::OneShot => Shape::OneShot {
            attack_ms: 10.0,
            hold_ms: 50.0,
            decay_ms: 200.0,
            decay_exp: 2,
        },
        ShapeKind::Sine => Shape::Sine {
            freq_hz: 2.0,
            depth_pct: 100.0,
        },
        ShapeKind::Sawtooth => Shape::Sawtooth {
            freq_hz: 2.0,
            depth_pct: 100.0,
        },
    }
}

fn step_shape(ui: &mut egui::Ui, cx: &EditorCtx, effect: &mut CustomEffect) {
    UiState::effect_card(ui, true, false, |ui| {
        ui.label(RichText::new(cx.t.step_fx_shape).strong().size(15.0));
        ui.add_space(4.0);

        let cur_kind = shape_kind_of(&effect.shape);
        let mut new_kind = None;
        egui::ComboBox::from_id_salt("fx_shape_select")
            .selected_text(shape_kind_label(cur_kind, cx.t))
            .show_ui(ui, |ui| {
                for k in SHAPE_KINDS {
                    if ui
                        .selectable_label(cur_kind == k, shape_kind_label(k, cx.t))
                        .clicked()
                    {
                        new_kind = Some(k);
                    }
                }
            });
        if let Some(k) = new_kind {
            effect.shape = default_shape_for_kind(k);
        }

        match &mut effect.shape {
            Shape::Constant => {}
            Shape::Pulse {
                freq_hz,
                duty_pct,
                jitter_pct,
                floor_pct,
                attack_ms,
            } => {
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_freq);
                    ui.add(egui::Slider::new(freq_hz, 0.2..=MAX_SHAPE_FREQ_HZ).suffix(" Hz"))
                        .on_hover_text(cx.t.hover_fx_freq);
                });
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_duty);
                    ui.add(egui::Slider::new(duty_pct, 0.0..=100.0).suffix("%"));
                });
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_jitter);
                    ui.add(egui::Slider::new(jitter_pct, 0.0..=100.0).suffix("%"));
                });
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_floor);
                    ui.add(egui::Slider::new(floor_pct, 0.0..=100.0).suffix("%"));
                });
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_attack);
                    ui.add(egui::Slider::new(attack_ms, 0.0..=1000.0).suffix(" ms"));
                });
            }
            Shape::OneShot {
                attack_ms,
                hold_ms,
                decay_ms,
                decay_exp,
            } => {
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_attack);
                    ui.add(egui::Slider::new(attack_ms, 0.0..=1000.0).suffix(" ms"));
                });
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_hold);
                    ui.add(egui::Slider::new(hold_ms, 0.0..=2000.0).suffix(" ms"));
                });
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_decay);
                    ui.add(egui::Slider::new(decay_ms, 0.0..=3000.0).suffix(" ms"));
                    ui.add(egui::DragValue::new(decay_exp).range(1..=5));
                });
            }
            Shape::Sine { freq_hz, depth_pct } => {
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_freq);
                    ui.add(egui::Slider::new(freq_hz, 0.2..=MAX_SHAPE_FREQ_HZ).suffix(" Hz"))
                        .on_hover_text(cx.t.hover_fx_freq);
                });
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_depth);
                    ui.add(egui::Slider::new(depth_pct, 0.0..=100.0).suffix("%"));
                });
            }
            Shape::Sawtooth { freq_hz, depth_pct } => {
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_freq);
                    ui.add(egui::Slider::new(freq_hz, 0.2..=MAX_SHAPE_FREQ_HZ).suffix(" Hz"))
                        .on_hover_text(cx.t.hover_fx_freq);
                });
                ui.horizontal(|ui| {
                    ui.label(cx.t.lbl_fx_depth);
                    ui.add(egui::Slider::new(depth_pct, 0.0..=100.0).suffix("%"));
                });
            }
        }
        // Пользователь мог вбить частоту вручную выше MAX_SHAPE_FREQ_HZ —
        // движок и так режет её на каждый тик (см. engine::shape_multiplier),
        // но лучше не показывать в UI значение, которое реально никогда не
        // будет отработано железом.
        effect.shape.clamp_freq();

        let raw = cx
            .live
            .as_ref()
            .and_then(|f| read_effect_value(effect, f))
            .unwrap_or_else(|| {
                let (lo, hi) = sources::def(effect.source).default_range;
                (lo + hi) / 2.0
            });
        let samples =
            engine::preview_waveform(effect, raw, OSCILLOSCOPE_WINDOW_S, OSCILLOSCOPE_SAMPLES);
        draw_oscilloscope(ui, &samples, OSCILLOSCOPE_HEIGHT);

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(cx.t.lbl_fx_strength);
            ui.add(egui::Slider::new(&mut effect.strength_pct, 0.0..=100.0).suffix("%"));
        });
        ui.horizontal(|ui| {
            ui.label(cx.t.lbl_fx_smoothing);
            ui.add(egui::Slider::new(&mut effect.smoothing_alpha, 0.0..=0.99));
        });
        ui.horizontal(|ui| {
            ui.label(cx.t.lbl_fx_mix);
            ui.radio_value(&mut effect.mix, MixMode::Max, cx.t.mix_max);
            ui.radio_value(&mut effect.mix, MixMode::Add, cx.t.mix_add);
        });
        ui.horizontal(|ui| {
            ui.label(cx.t.lbl_fx_output);
            ui.checkbox(&mut effect.out.joystick, cx.t.lbl_fx_out_joystick);
            ui.checkbox(&mut effect.out.throttle_left, cx.t.lbl_fx_out_throttle_left);
            ui.checkbox(
                &mut effect.out.throttle_right,
                cx.t.lbl_fx_out_throttle_right,
            );
        });
    });
}

fn show_warnings(ui: &mut egui::Ui, cx: &EditorCtx, effect: &CustomEffect) {
    let no_output = !(effect.out.joystick || effect.out.throttle_left || effect.out.throttle_right);
    if no_output {
        ui.colored_label(palette::STATUS_WARN, cx.t.warn_fx_no_output);
    }
    let always_on = matches!(effect.trigger, Trigger::Always)
        && matches!(effect.shape, Shape::Constant)
        && effect.strength_pct > 80.0;
    if always_on {
        ui.colored_label(palette::STATUS_WARN, cx.t.warn_fx_always_on);
    }
}

/// Живой предпросмотр эффекта прямо на моторах — два независимых источника
/// сырого значения на один и тот же канал (см. `PreviewSource`):
/// 1) пользователь двигает "пробное значение" рукой и жмёт «Играть» — то же
///    самое, что дал бы движок при таком значении источника в реальном
///    полёте/бою, только без самого полёта/боя;
/// 2) Задача B — загруженная запись (`btn_fx_open_session`) целиком
///    проигрывается в реальном времени (`btn_fx_play_session`), с графиком
///    офлайн-прогона и скрабом курсора мышью.
///
/// Про освобождение канала (`PreviewLock`) — см. `EditorState::stop_preview`:
/// это ЕДИНСТВЕННЫЙ путь остановки, вызывается отсюда (кнопки «Стоп» обоих
/// режимов) и из трёх других мест в этом модуле + страховки в `ui.rs`.
fn step_preview(
    ui: &mut egui::Ui,
    st: &mut EditorState,
    cx: &EditorCtx,
    effect: &mut CustomEffect,
) {
    UiState::effect_card(ui, true, false, |ui| {
        ui.label(RichText::new(cx.t.heading_fx_preview).strong().size(15.0));
        ui.add_space(4.0);

        let def = sources::def(effect.source);
        // Диапазон слайдера берём из КРИВОЙ эффекта (тот же `curve_x_bounds`,
        // по которому нарисована ось X в шаге 3), а не из статического
        // `default_range` источника. У `Lvar` этот статический диапазон —
        // заглушка 0..1, потому что реальные единицы своей переменной знает
        // только пользователь: слайдер стоял в 0..1, кривая жила в 0..400, и
        // предпросмотр честно отдавал на моторы ноль при любом положении
        // ползунка — выглядело как "кнопка не работает".
        let (lo, hi) = curve_x_bounds(&effect.curve, def.default_range);
        if st.test_value_source != Some(effect.source)
            || st.test_value_effect.as_deref() != Some(effect.id.as_str())
        {
            // Другой эффект или другой источник — старое пробное значение было
            // бы в чужих единицах/диапазоне, начинаем с середины нового.
            st.test_value = (lo + hi) / 2.0;
            st.test_value_source = Some(effect.source);
            st.test_value_effect = Some(effect.id.clone());
        } else {
            // Тот же эффект, но пользователь мог утащить точку кривой за
            // прежние границы прямо сейчас — не сбрасываем значение (это
            // дёргало бы ползунок посреди перетаскивания), только вводим его
            // обратно в допустимые пределы.
            st.test_value = st.test_value.clamp(lo.min(hi), lo.max(hi));
        }
        ui.horizontal(|ui| {
            ui.label(cx.t.lbl_fx_test_value);
            let mut slider = egui::Slider::new(&mut st.test_value, lo.min(hi)..=lo.max(hi));
            if def.kind == SourceKind::Boolean {
                // Булев источник — только 0/1 имеют смысл (см. IsTrue), а не
                // произвольная дробь между ними.
                slider = slider.step_by(1.0);
            }
            ui.add(slider);
        });

        // `is_playing_this` — идёт ЛЮБОЙ предпросмотр (неважно, из какого
        // источника) именно для ЭТОГО эффекта; is_test_playing/
        // is_session_playing уточняют, из какого — только они решают, какая
        // из двух кнопок ниже показывает "Стоп".
        let is_playing_this = st.preview_effect_id.as_deref() == Some(effect.id.as_str());
        let is_test_playing = is_playing_this && st.preview_source == PreviewSource::TestValue;
        let is_session_playing = is_playing_this && st.preview_source == PreviewSource::Session;

        ui.horizontal(|ui| {
            let label = if is_test_playing {
                cx.t.btn_fx_stop
            } else {
                cx.t.btn_fx_play
            };
            if ui.button(label).clicked() {
                if is_test_playing {
                    st.stop_preview(cx.tx_hid, cx.preview);
                } else {
                    // На случай, если играл ДРУГОЙ эффект ИЛИ этот же эффект,
                    // но в режиме сессии — сначала честно гасим предыдущий,
                    // потом захватываем канал заново под ручной слайдер.
                    st.stop_preview(cx.tx_hid, cx.preview);
                    cx.preview.take();
                    st.preview_effect_id = Some(effect.id.clone());
                    st.preview_source = PreviewSource::TestValue;
                    st.preview_started_at = Instant::now();
                    // Шлём первый кадр немедленно, не дожидаясь троттлинга.
                    st.preview_last_sent = Instant::now() - PREVIEW_SEND_INTERVAL;
                }
            }
        });

        if is_test_playing {
            let now = Instant::now();
            if now.saturating_duration_since(st.preview_last_sent) >= PREVIEW_SEND_INTERVAL {
                let elapsed = st.preview_started_at.elapsed().as_secs_f64();
                let t = preview_playback_time_s(effect, elapsed);
                let (joystick, throttle_left, throttle_right) =
                    engine::preview_level(effect, st.test_value, t);
                st.preview_last_levels = (joystick, throttle_left, throttle_right);
                let _ = cx.tx_hid.send(HidCmd::SendIntensity {
                    joystick,
                    throttle_left,
                    throttle_right,
                });
                st.preview_last_sent = now;
            }
            // Без этого egui перерисовывал бы окно только по вводу мыши/
            // клавиатуры, и предпросмотр застыл бы на статичной картинке
            // между реальными событиями ввода.
            ui.ctx().request_repaint();
        }

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let (j, tl, tr) = st.preview_last_levels;
            ui.label(cx.t.lbl_fx_out_joystick);
            ui.label(RichText::new(j.to_string()).monospace().strong());
            ui.separator();
            ui.label(cx.t.lbl_fx_out_throttle_left);
            ui.label(RichText::new(tl.to_string()).monospace().strong());
            ui.separator();
            ui.label(cx.t.lbl_fx_out_throttle_right);
            ui.label(RichText::new(tr.to_string()).monospace().strong());
        });

        // Событийные эффекты (OneShot/Changed) в реальной игре срабатывают
        // один раз по фронту (см. `preview_playback_time_s` ниже, ровно тот
        // же критерий `is_event_effect`) — без этой подписи зацикленный
        // предпросмотр выглядел бы как баг ("почему тут само по себе
        // повторяется"), хотя в игре так себя эффект вести не будет.
        if is_event_effect(effect) {
            ui.add_space(2.0);
            ui.label(
                RichText::new(cx.t.lbl_fx_preview_loops_events)
                    .italics()
                    .color(palette::TEXT_DISABLED),
            );
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        step_session(ui, st, cx, effect, is_session_playing);
    });
}

/// Задача B: прогон записанной сессии по текущему эффекту. Вынесен из
/// `step_preview` отдельной функцией — сама карточка и так не маленькая,
/// а тут ещё офлайн-пересчёт ряда + скраб + запуск воспроизведения.
fn step_session(
    ui: &mut egui::Ui,
    st: &mut EditorState,
    cx: &EditorCtx,
    effect: &mut CustomEffect,
    is_session_playing: bool,
) {
    ui.horizontal(|ui| {
        if ui.button(cx.t.btn_fx_open_session).clicked() {
            let start_dir = player::sessions_dir();
            if let Some(path) = file_dialog::open_file(
                cx.t.dlg_fx_session_title,
                cx.t.filter_session_file,
                "jsonl",
                Some(&start_dir),
            ) {
                match player::load_session(&path) {
                    Ok(session) => {
                        cx.logs.push(format!(
                            "Custom FX: loaded session {} ({} frame(s), {:.1}s)",
                            session.source_path.display(),
                            session.frames.len(),
                            session.duration_s
                        ));
                        st.session = Some(session);
                        // Новая запись — старый офлайн-прогон и позиция
                        // курсора относятся к прошлому файлу, протухли.
                        st.session_series = None;
                        st.session_cursor_s = 0.0;
                        // Загрузка новой записи, пока играет прошлая сессия
                        // этого же эффекта — гасим воспроизведение, иначе
                        // оно продолжило бы шагать по УЖЕ ЗАМЕНЁННОМУ session
                        // (следующий тик step_session ниже прочитал бы кадры
                        // новой записи под старым progress-курсором).
                        if is_session_playing {
                            st.stop_preview(cx.tx_hid, cx.preview);
                        }
                    }
                    Err(e) => cx
                        .logs
                        .push(format!("Custom FX: failed to load session: {e}")),
                }
            }
        }

        let session_ready = st.session.as_ref().is_some_and(|s| !s.frames.is_empty());
        let label = if is_session_playing {
            cx.t.btn_fx_stop
        } else {
            cx.t.btn_fx_play_session
        };
        if ui
            .add_enabled(session_ready, egui::Button::new(label))
            .clicked()
        {
            if is_session_playing {
                st.stop_preview(cx.tx_hid, cx.preview);
            } else {
                st.stop_preview(cx.tx_hid, cx.preview);
                cx.preview.take();
                st.preview_effect_id = Some(effect.id.clone());
                st.preview_source = PreviewSource::Session;
                // Резюмируем с текущей позиции скраба, а не всегда с нуля —
                // так перетащенный мышью курсор и кнопка "Играть" согласованы.
                let resume_from = st.session_cursor_s.max(0.0);
                st.preview_started_at = Instant::now() - Duration::from_secs_f64(resume_from);
                st.preview_last_sent = Instant::now() - PREVIEW_SEND_INTERVAL;
            }
        }
    });

    let Some(session) = st.session.as_ref() else {
        ui.label(RichText::new(cx.t.lbl_fx_session_none).color(palette::TEXT_DISABLED));
        return;
    };

    // Задача 1: на месте "Запись не загружена" после успешной загрузки
    // должно быть видно, КАКАЯ запись открыта — на диске у пользователя
    // десятки файлов вида session_20260803_101812.jsonl, различимых только
    // по имени. Полный путь длинный и ломает вёрстку, поэтому уходит в
    // hover, а на виду — только имя файла + длительность + число кадров.
    let file_name = session
        .source_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| session.source_path.display().to_string());
    let session_info =
        cx.t.lbl_fx_session_loaded
            .replace("{name}", &file_name)
            .replace("{frames}", &session.frames.len().to_string())
            .replace("{dur:.1}", &format!("{:.1}", session.duration_s));
    ui.label(session_info)
        .on_hover_text(session.source_path.display().to_string());

    // Офлайн-прогон всей записи — единственная реальная "тяжёлая" часть
    // Задачи B (тысячи кадров через engine::preview_level). Пересчитываем
    // НЕ каждый кадр GUI, а только когда реально сменились либо запись,
    // либо сам эффект (сравнение всей структуры — она Clone+PartialEq).
    let needs_recompute = match &st.session_series {
        Some(series) => {
            series.for_session_path != session.source_path || series.for_effect != *effect
        }
        None => true,
    };
    if needs_recompute {
        st.session_series = Some(recompute_session_series(effect, session));
    }

    // Задача 2: если запись загружена (не пустая, не битая), но источник
    // эффекта не дал сигнала НИ НА ОДНОМ её кадре — это не "нет сигнала", а
    // "запись сделана в другой игре" (см. `sources::read`: `None` для
    // источника чужого конвейера — штатное поведение). Флаг посчитан ОДИН
    // РАЗ вместе с остальным офлайн-прогоном в `recompute_session_series`
    // (кэш `session_series` выше), а не заново каждый кадр GUI.
    let no_signal_text = match st.session_series.as_ref() {
        Some(series) if series.game_mismatch => {
            let source_games = sources::def(effect.source).games;
            let rec = session
                .frames
                .first()
                .map(|sf| telemetry_game_label(&sf.frame, cx.t))
                .unwrap_or_default();
            let src = game_mask_label(source_games, cx.t);
            cx.t.lbl_fx_session_game_mismatch
                .replace("{rec}", &rec)
                .replace("{src}", &src)
        }
        _ => cx.t.lbl_fx_no_signal.to_string(),
    };

    let t0 = session.frames.first().map(|f| f.t).unwrap_or(0.0);
    let cursor_before = st.session_cursor_s;
    draw_session_graph(
        ui,
        session,
        t0,
        st.session_series.as_ref(),
        &mut st.session_cursor_s,
        &no_signal_text,
    );
    // Скраб мышью во время воспроизведения — переносим "начало отсчёта"
    // проигрывания на новую позицию, иначе на следующем тике курсор дёрнулся
    // бы обратно туда, куда его довело реальное время с СТАРОГО старта.
    if is_session_playing && (st.session_cursor_s - cursor_before).abs() > f64::EPSILON {
        st.preview_started_at =
            Instant::now() - Duration::from_secs_f64(st.session_cursor_s.max(0.0));
        st.preview_last_sent = Instant::now() - PREVIEW_SEND_INTERVAL;
    }

    if is_session_playing {
        let now = Instant::now();
        if now.saturating_duration_since(st.preview_last_sent) >= PREVIEW_SEND_INTERVAL {
            let t = st.preview_started_at.elapsed().as_secs_f64();
            let finished = t >= session.duration_s;
            st.session_cursor_s = t.min(session.duration_s.max(0.0));

            // Как и офлайн-прогон ниже — берём "действующий на момент t"
            // кадр записи и честно гасим канал нулями, если у него нет
            // значения для источника эффекта (другая игра), а не подставляем
            // фейковый ноль как настоящее показание.
            let levels = frame_at_relative_time(&session.frames, t0, t)
                .and_then(|sf| read_effect_value(effect, &sf.frame))
                .map(|raw| engine::preview_level(effect, raw, t))
                .unwrap_or((0, 0, 0));
            st.preview_last_levels = levels;
            let _ = cx.tx_hid.send(HidCmd::SendIntensity {
                joystick: levels.0,
                throttle_left: levels.1,
                throttle_right: levels.2,
            });
            st.preview_last_sent = now;

            if finished {
                st.stop_preview(cx.tx_hid, cx.preview);
            }
        }
        ui.ctx().request_repaint();
    }
}

/// Офлайн-прогон эффекта по ВСЕЙ записи — по кадру `sources::read` берёт
/// сырое значение (или честно `None`, если источник эффекта не пишется в
/// этой записи — правило Задачи B, "не подставляй нули как настоящие
/// значения"), а из него `engine::preview_level` считает три канала на
/// момент этого кадра. `t`, который получает `preview_level`, — время
/// кадра ОТНОСИТЕЛЬНО начала записи: он определяет фазу периодических форм
/// (Pulse/Sine/Sawtooth) и таймер OneShot так же, как это было бы при живом
/// проигрывании с самого начала записи.
fn recompute_session_series(effect: &CustomEffect, session: &Session) -> SessionSeries {
    let t0 = session.frames.first().map(|f| f.t).unwrap_or(0.0);
    let levels = session
        .frames
        .iter()
        .map(|sf| {
            read_effect_value(effect, &sf.frame)
                .map(|raw| engine::preview_level(effect, raw, sf.t - t0))
        })
        .collect();
    // Задача 2: считаем СРАЗУ здесь, одним проходом вместе с `levels`, а не
    // отдельным пересчётом по требованию — та же запись уже итерируется.
    let source_games = sources::def(effect.source).games;
    let game_mismatch = !session.frames.is_empty()
        && session
            .frames
            .iter()
            .all(|sf| frame_is_other_game(&sf.frame, source_games));
    SessionSeries {
        levels,
        for_session_path: session.source_path.clone(),
        for_effect: effect.clone(),
        game_mismatch,
    }
}

/// Кадр записи принадлежит ДРУГОЙ игре, чем `source_games` источника
/// эффекта — чистая структурная проверка по варианту `TelemetryFrame`, без
/// похода в `sources::read`. Используется, чтобы отличить "запись из другой
/// игры" (Задача 2) от обычного "нет сигнала".
fn frame_is_other_game(frame: &TelemetryFrame, source_games: GameMask) -> bool {
    match frame {
        TelemetryFrame::Flight(_) => !(source_games.msfs || source_games.xplane),
        TelemetryFrame::Wt(_) => !source_games.wt,
    }
}

/// Локализованное название игры, которой принадлежит кадр записи —
/// `TelemetryFrame::Flight` не различает MSFS и X-Plane (оба сходятся в
/// одном `FlightVars`), поэтому называет обе разом.
fn telemetry_game_label(frame: &TelemetryFrame, t: &Strings) -> String {
    match frame {
        TelemetryFrame::Wt(_) => t.hover_game_wt.to_string(),
        TelemetryFrame::Flight(_) => format!("{}/{}", t.hover_game_msfs, t.hover_game_xp),
    }
}

/// Локализованный список игр, которым разрешён `GameMask` — те же названия,
/// что уже показывает чек-лист "Игры" на шаге 3 (`hover_game_msfs`/
/// `hover_game_xp`/`hover_game_wt`). Не полагается на то, что маска source
/// сегодня всегда либо чисто Flight, либо чисто Wt (см.
/// `flight_and_wt_sources_are_mutually_exclusive_masks` в sources.rs) —
/// просто перечисляет включённые флаги.
fn game_mask_label(games: GameMask, t: &Strings) -> String {
    let mut names = Vec::new();
    if games.msfs {
        names.push(t.hover_game_msfs);
    }
    if games.xplane {
        names.push(t.hover_game_xp);
    }
    if games.wt {
        names.push(t.hover_game_wt);
    }
    names.join("/")
}

/// Кадр записи, действующий на момент `t_rel` секунд от начала записи —
/// "последний кадр, чьё время не позже запрошенного" (действующее
/// состояние не может прийти из будущего, та же семантика, что у живой
/// телеметрии). `frames` обязаны быть отсортированы по `t` по возрастанию —
/// это гарантирует `player::load_session`.
fn frame_at_relative_time(frames: &[SessionFrame], t0: f64, t_rel: f64) -> Option<&SessionFrame> {
    if frames.is_empty() {
        return None;
    }
    let idx = frames.partition_point(|f| f.t - t0 <= t_rel);
    if idx == 0 {
        frames.first()
    } else {
        frames.get(idx - 1)
    }
}

// ---------------------------------------------------------------------
// Графики
// ---------------------------------------------------------------------

/// Мини-график живой истории значения (шаги 1 и 2): ось X — время (справа
/// "сейчас", слева — `LIVE_HISTORY_SECONDS` назад), ось Y — значение
/// источника в его собственных единицах. `overlay` рисует порог/зону
/// триггера — горизонтально, потому что порог здесь всегда порог ПО
/// ЗНАЧЕНИЮ, а значение на этом графике живёт на оси Y, а не на оси X (в
/// отличие от холста кривой отклика в шаге 3, где как раз наоборот).
/// Приглушённая надпись по центру пустой рамки графика (A3) — общий хвост
/// для мини-графиков шагов 1/2 и графика записи (Задача B): "рамка пустая"
/// иначе неотличимо от "рамка сломана".
fn draw_no_signal_placeholder(painter: &egui::Painter, rect: Rect, text: &str) {
    // Переносим текст по ширине панели, а не рисуем одной строкой: короткое
    // "нет сигнала" помещается всегда, но сюда же приходит объяснение про
    // несовпадение игры записи и источника эффекта — целое предложение,
    // которое painter.text() (без переноса, центрирование по rect) обрезал бы
    // сразу с ОБОИХ краёв, оставив нечитаемый огрызок.
    let galley = painter.layout(
        text.to_owned(),
        egui::FontId::proportional(11.0),
        palette::TEXT_DISABLED,
        (rect.width() - 16.0).max(32.0),
    );
    let pos = rect.center() - galley.size() / 2.0;
    painter.galley(pos, galley, palette::TEXT_DISABLED);
}

fn draw_mini_graph(
    ui: &mut egui::Ui,
    history: &VecDeque<(Instant, f32)>,
    bounds: (f64, f64),
    height: f32,
    overlay: TriggerOverlay,
    no_signal_text: &str,
) {
    let desired = Vec2::new(ui.available_width(), height);
    let (rect, _resp) = ui.allocate_exact_size(desired, Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 3u8, palette::BG_APP);
    painter.rect_stroke(
        rect,
        3u8,
        egui::Stroke::new(1.0, palette::BORDER_DEFAULT),
        egui::StrokeKind::Inside,
    );

    // A3: раньше при отсутствии сигнала рамка оставалась совсем пустой —
    // на реальном прогоне (игра не запущена/борт не даёт этот источник)
    // выглядело как поломка, а не как штатное "сигнала пока нет".
    if history.is_empty() {
        draw_no_signal_placeholder(&painter, rect, no_signal_text);
    }

    match overlay {
        TriggerOverlay::Band(lo, hi) => {
            let y0 = value_to_px_y(lo, bounds, rect);
            let y1 = value_to_px_y(hi, bounds, rect);
            let band = Rect::from_min_max(
                egui::pos2(rect.left(), y0.min(y1).max(rect.top())),
                egui::pos2(rect.right(), y0.max(y1).min(rect.bottom())),
            );
            painter.rect_filled(band, 0u8, palette::SELECTED_BG.gamma_multiply(0.7));
        }
        TriggerOverlay::Line(v) => {
            let y = value_to_px_y(v, bounds, rect).clamp(rect.top(), rect.bottom());
            painter.line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                egui::Stroke::new(1.5, palette::STATUS_WARN),
            );
        }
        TriggerOverlay::None => {}
    }

    if !history.is_empty() {
        let now = Instant::now();
        let points: Vec<egui::Pos2> = history
            .iter()
            .map(|(t, v)| {
                let age = now.saturating_duration_since(*t).as_secs_f32();
                let x = map_range(
                    age,
                    LIVE_HISTORY_SECONDS as f32,
                    0.0,
                    rect.left(),
                    rect.right(),
                )
                .clamp(rect.left(), rect.right());
                let y = value_to_px_y(*v as f64, bounds, rect).clamp(rect.top(), rect.bottom());
                egui::pos2(x, y)
            })
            .collect();
        for w in points.windows(2) {
            painter.line_segment([w[0], w[1]], egui::Stroke::new(1.5, palette::ACCENT_LIVE));
        }
        if let Some(&last) = points.last() {
            painter.circle_filled(last, 2.5, palette::ACCENT_LIVE);
        }
    }
}

/// Осциллограф формы (шаг 4): 2-секундное окно `preview_waveform` (0..255) в
/// виде столбиков от нижнего края холста. Столбики, а не заливка одним
/// многоугольником через `egui::Shape::convex_polygon` — у эпейнтовского
/// convex_polygon веерная триангуляция от первой вершины, которая ломается
/// (даёт неверную заливку) на НЕ звёздчатых относительно этой вершины
/// фигурах — а колебания вроде Sine/Sawtooth на пару герц за 2 секунды дают
/// именно такую, многократно ныряющую к нулю фигуру. Столбики корректны при
/// любой форме сигнала и всё равно читаются как "залитая кривая".
fn draw_oscilloscope(ui: &mut egui::Ui, samples: &[f32], height: f32) {
    let desired = Vec2::new(ui.available_width(), height);
    let (rect, _resp) = ui.allocate_exact_size(desired, Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 3u8, palette::BG_APP);
    painter.rect_stroke(
        rect,
        3u8,
        egui::Stroke::new(1.0, palette::BORDER_DEFAULT),
        egui::StrokeKind::Inside,
    );

    if samples.is_empty() {
        return;
    }
    let n = samples.len();
    for (i, &s) in samples.iter().enumerate() {
        let frac = (s / 255.0).clamp(0.0, 1.0);
        let bar_h = frac * rect.height();
        // Границы столбика — от долей ШИРИНЫ ХОЛСТА (x0/x1 через i/n), а не
        // накоплением одинаковой округлённой ширины (rect.left() + i*bar_w):
        // при накоплении ошибка округления bar_w копится с каждым столбиком и
        // у правого края полосы соседние столбики перестают смыкаться —
        // видимые разрывы в заливке. Так x1 текущего столбика всегда равен x0
        // следующего (оба считаются по одной и той же формуле от i/n).
        let x0 = rect.left() + rect.width() * i as f32 / n as f32;
        let x1 = rect.left() + rect.width() * (i + 1) as f32 / n as f32;
        let bar = Rect::from_min_max(
            egui::pos2(x0, rect.bottom() - bar_h),
            egui::pos2(x1, rect.bottom()),
        );
        painter.rect_filled(bar, 0u8, palette::ACCENT_LIVE.gamma_multiply(0.85));
    }
}

/// Задача B: график офлайн-прогона записи. Ось X — время записи (0 —
/// начало, `session.duration_s` — конец), ось Y — интенсивность 0..255,
/// три канала — тремя тонами из `palette`. Интерактивен: вся площадь
/// холста ловит click_and_drag, и позиция под курсором становится новой
/// позицией курсора воспроизведения (скраб) — общая арифметика "время <->
/// пиксель" с холстом кривой (шаг 3) через те же `value_to_px_x`/
/// `px_x_to_value`, никакого дублирования.
fn draw_session_graph(
    ui: &mut egui::Ui,
    session: &Session,
    t0: f64,
    series: Option<&SessionSeries>,
    cursor_s: &mut f64,
    no_signal_text: &str,
) {
    let bounds = session_x_bounds(session.duration_s);
    let y_bounds = (0.0, 255.0);

    let desired = Vec2::new(ui.available_width(), SESSION_GRAPH_HEIGHT);
    // Скраб мышью доступен, только пока в записи реально есть кадры — иначе
    // (пустая запись) курсор дёргался бы по клику/драгу без единой точки
    // данных под ним, как и активная кнопка "Играть" без загруженной записи
    // (см. `session_ready` в `step_session` выше).
    let sense = if session.frames.is_empty() {
        Sense::hover()
    } else {
        Sense::click_and_drag()
    };
    let (rect, response) = ui.allocate_exact_size(desired, sense);
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 3u8, palette::BG_APP);
    painter.rect_stroke(
        rect,
        3u8,
        egui::Stroke::new(1.0, palette::BORDER_DEFAULT),
        egui::StrokeKind::Inside,
    );

    // Три канала — три тона, уже используемых где-то ещё в модуле для
    // живых индикаций (ACCENT_LIVE/STATUS_OK/STATUS_ATTENTION), а не новые
    // константы palette: задача прямо требует брать тона ТОЛЬКО из palette,
    // и этих трёх достаточно, чтобы линии не сливались друг с другом на
    // тёмном фоне.
    const CHANNEL_COLORS: [Color32; 3] = [
        palette::ACCENT_LIVE,
        palette::STATUS_OK,
        palette::STATUS_ATTENTION,
    ];

    let mut any_signal = false;
    if let Some(series) = series {
        // Рисуем ломаную КАЖДОГО канала отдельно, разрывая её на границе
        // любого None-кадра (другая игра — см. `recompute_session_series`)
        // — иначе прямая линия через дыру соврала бы, будто там есть данные.
        for (ch, &color) in CHANNEL_COLORS.iter().enumerate() {
            let mut prev: Option<egui::Pos2> = None;
            for (sf, lv) in session.frames.iter().zip(series.levels.iter()) {
                let Some(levels) = lv else {
                    prev = None;
                    continue;
                };
                any_signal = true;
                let v = match ch {
                    0 => levels.0,
                    1 => levels.1,
                    _ => levels.2,
                };
                let x = value_to_px_x(sf.t - t0, bounds, rect);
                let y = value_to_px_y(v as f64, y_bounds, rect);
                let p = egui::pos2(x, y);
                if let Some(prev_p) = prev {
                    painter.line_segment([prev_p, p], egui::Stroke::new(1.5, color));
                }
                prev = Some(p);
            }
        }
    }
    if !any_signal {
        draw_no_signal_placeholder(&painter, rect, no_signal_text);
    }

    // Курсор воспроизведения/скраба — вертикальная линия поверх графика.
    let cursor_x = value_to_px_x(*cursor_s, bounds, rect).clamp(rect.left(), rect.right());
    painter.line_segment(
        [
            egui::pos2(cursor_x, rect.top()),
            egui::pos2(cursor_x, rect.bottom()),
        ],
        egui::Stroke::new(1.5, palette::TEXT_PRIMARY),
    );

    // Скраб: клик или драг где угодно по холсту передвигает курсор.
    if (response.dragged() || response.clicked())
        && let Some(pos) = response.interact_pointer_pos()
    {
        *cursor_s = px_x_to_value(pos.x, bounds, rect).clamp(bounds.0, bounds.1);
    }
}

/// Диапазон оси X графика записи: 0..`duration_s`. Вырожденная (нулевая
/// длительность — например, запись из одного кадра) сессия не должна
/// ломать `map_range` делением на ноль, тот же приём, что `curve_x_bounds`.
fn session_x_bounds(duration_s: f64) -> (f64, f64) {
    if duration_s <= 0.0 {
        (0.0, 1.0)
    } else {
        (0.0, duration_s)
    }
}

// ---------------------------------------------------------------------
// Мелкие чистые помощники (покрыты юнит-тестами ниже)
// ---------------------------------------------------------------------

/// Эффект "событийный" — срабатывает по фронту и естественным образом гаснет
/// сам, а не удерживается непрерывно: `Shape::OneShot` (огибающая
/// attack->hold->decay) или `Trigger::Changed` (импульс на изменение,
/// держится ровно `hold_s`). В реальной игре (`CustomFxEngine::step`) такой
/// эффект честно срабатывает один раз на событие — поэтому его РУЧНОЙ
/// предпросмотр обязан зацикливаться (`preview_playback_time_s`), иначе после
/// первого же удара индикаторы навсегда повисают на нуле, а кнопка
/// продолжает показывать "Стоп", как будто что-то ещё играет.
fn is_event_effect(effect: &CustomEffect) -> bool {
    matches!(effect.shape, Shape::OneShot { .. })
        || matches!(effect.trigger, Trigger::Changed { .. })
}

/// Длительность одного события эффекта, секунды — максимум из полной
/// огибающей `Shape::OneShot` (attack+hold+decay) и `hold_s` у
/// `Trigger::Changed`. Оба поля независимы друг от друга (эффект может иметь
/// OneShot-форму без Changed-триггера или наоборот), отсюда максимум, а не
/// сумма и не выбор одного варианта.
fn event_duration_s(effect: &CustomEffect) -> f64 {
    let mut duration = 0.0f64;
    if let Shape::OneShot {
        attack_ms,
        hold_ms,
        decay_ms,
        ..
    } = effect.shape
    {
        duration = duration.max((attack_ms as f64 + hold_ms as f64 + decay_ms as f64) / 1000.0);
    }
    if let Trigger::Changed { hold_s, .. } = effect.trigger {
        duration = duration.max(hold_s);
    }
    duration
}

/// Время, которое реально подаётся в `engine::preview_level` на тике ручного
/// предпросмотра — ЧИСТАЯ функция (без egui/Instant), вынесена отдельно ради
/// юнит-тестов. Для событийных эффектов (`is_event_effect`) заворачивает
/// `elapsed_s` по модулю периода (длительность события + пауза
/// `PREVIEW_EVENT_REPEAT_GAP_S`) — так `preview_level` раз за разом видит
/// свежий фронт "молчал -> активен" в t=0 и удар повторяется циклически. Для
/// остальных форм/триггеров (Constant/Pulse/Sine/Sawtooth с непрерывным
/// триггером) возвращает `elapsed_s` без изменений — там `t` обязан расти
/// монотонно, иначе поедет фаза периодического сигнала.
fn preview_playback_time_s(effect: &CustomEffect, elapsed_s: f64) -> f64 {
    if !is_event_effect(effect) {
        return elapsed_s;
    }
    let period = event_duration_s(effect) + PREVIEW_EVENT_REPEAT_GAP_S;
    if period <= 0.0 {
        return elapsed_s;
    }
    elapsed_s.rem_euclid(period)
}

fn default_source_for_game(game: ActiveGame) -> SourceId {
    match game {
        ActiveGame::Wt => SourceId::WtIasKmh,
        _ => SourceId::FlightAirspeedKn,
    }
}

/// Группа источника в выпадающем списке "1. Источник" — к какой игре он
/// относится. Определяется маской `SourceDef::games`, а не позицией строки
/// в `SOURCES`: так заголовок группы сам появится над новым источником,
/// когда его добавят в таблицу, без правки этого файла. Три варианта
/// соответствуют ровно трём маскам, которые сейчас использует таблица
/// `SOURCES` (`FLIGHT_ONLY`/`WT_ONLY`/`MSFS_ONLY` в `custom_fx/sources.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceGroup {
    /// MSFS + X-Plane (общий конвейер `FlightVars`).
    Flight,
    /// War Thunder.
    Wt,
    /// Пользовательская LVAR — физически существует только в MSFS.
    MsfsLvar,
}

fn source_group(games: GameMask) -> SourceGroup {
    if games.wt {
        SourceGroup::Wt
    } else if games.xplane {
        SourceGroup::Flight
    } else {
        SourceGroup::MsfsLvar
    }
}

fn source_group_header(group: SourceGroup, t: &Strings) -> &'static str {
    match group {
        SourceGroup::Flight => t.hdr_fx_source_group_flight,
        SourceGroup::Wt => t.hdr_fx_source_group_wt,
        SourceGroup::MsfsLvar => t.hdr_fx_source_group_lvar,
    }
}

fn source_label(def: &SourceDef, lang: Lang) -> String {
    let name = match lang {
        Lang::En => def.label_en,
        Lang::Ru => def.label_ru,
    };
    if def.unit.is_empty() {
        name.to_string()
    } else {
        format!("{name} ({})", def.unit)
    }
}

/// A4: делает `base` уникальным среди имён уже существующих эффектов —
/// добавляет порядковый номер, начиная с 2 ("Airspeed (IAS) (kn)" ->
/// "Airspeed (IAS) (kn) 2"), если `base` уже занято. Первый эффект с таким
/// именем суффикса не получает — только начиная со второго совпадения.
fn unique_effect_name(base: &str, effects: &[CustomEffect]) -> String {
    let taken = |candidate: &str| effects.iter().any(|e| e.name == candidate);
    if !taken(base) {
        return base.to_string();
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base} {n}");
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Диапазон оси X холста кривой отклика: базовый диапазон источника,
/// расширенный, если пользователь руками утащил точку за его пределы —
/// иначе такую точку было бы физически не видно (и не за что схватить).
fn curve_x_bounds(curve: &ResponseCurve, default_range: (f64, f64)) -> (f64, f64) {
    let (mut lo, mut hi) = default_range;
    for p in &curve.points {
        lo = lo.min(p.x);
        hi = hi.max(p.x);
    }
    if hi <= lo {
        hi = lo + 1.0;
    }
    (lo, hi)
}

/// Линейно отображает `value` из `[in_lo, in_hi]` в `[out_lo, out_hi]` —
/// общий примитив перевода "значение <-> пиксель" для всех трёх графиков
/// модуля. Вырожденный входной диапазон (`in_hi == in_lo`) не делит на
/// ноль — курсор мыши должен попадать хоть куда-то даже на кривой из одной
/// точки, поэтому возвращаем середину выходного диапазона.
fn map_range(value: f32, in_lo: f32, in_hi: f32, out_lo: f32, out_hi: f32) -> f32 {
    if (in_hi - in_lo).abs() < f32::EPSILON {
        return (out_lo + out_hi) * 0.5;
    }
    let t = (value - in_lo) / (in_hi - in_lo);
    out_lo + t * (out_hi - out_lo)
}

fn value_to_px_x(value: f64, bounds: (f64, f64), rect: Rect) -> f32 {
    map_range(
        value as f32,
        bounds.0 as f32,
        bounds.1 as f32,
        rect.left(),
        rect.right(),
    )
}

fn px_x_to_value(px: f32, bounds: (f64, f64), rect: Rect) -> f64 {
    map_range(
        px,
        rect.left(),
        rect.right(),
        bounds.0 as f32,
        bounds.1 as f32,
    ) as f64
}

/// Экранные Y растут вниз, поэтому в отличие от X здесь домены на выходе
/// развёрнуты: `bounds.1` (максимум значения) попадает НАВЕРХ холста.
fn value_to_px_y(value: f64, bounds: (f64, f64), rect: Rect) -> f32 {
    map_range(
        value as f32,
        bounds.0 as f32,
        bounds.1 as f32,
        rect.bottom(),
        rect.top(),
    )
}

fn px_y_to_value(px: f32, bounds: (f64, f64), rect: Rect) -> f64 {
    map_range(
        px,
        rect.bottom(),
        rect.top(),
        bounds.0 as f32,
        bounds.1 as f32,
    ) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_rect() -> Rect {
        Rect::from_min_max(egui::pos2(100.0, 200.0), egui::pos2(300.0, 260.0))
    }

    #[test]
    fn map_range_identity_at_endpoints() {
        assert!((map_range(0.0, 0.0, 10.0, 0.0, 100.0) - 0.0).abs() < 1e-4);
        assert!((map_range(10.0, 0.0, 10.0, 0.0, 100.0) - 100.0).abs() < 1e-4);
        assert!((map_range(5.0, 0.0, 10.0, 0.0, 100.0) - 50.0).abs() < 1e-4);
    }

    #[test]
    fn map_range_handles_reversed_output_range() {
        // Экранные координаты Y растут вниз — типичный случай, когда
        // out_lo > out_hi.
        assert!((map_range(0.0, 0.0, 1.0, 260.0, 200.0) - 260.0).abs() < 1e-4);
        assert!((map_range(1.0, 0.0, 1.0, 260.0, 200.0) - 200.0).abs() < 1e-4);
        assert!((map_range(0.5, 0.0, 1.0, 260.0, 200.0) - 230.0).abs() < 1e-4);
    }

    #[test]
    fn map_range_degenerate_input_returns_output_midpoint() {
        let v = map_range(42.0, 5.0, 5.0, 10.0, 20.0);
        assert!((v - 15.0).abs() < 1e-4);
    }

    #[test]
    fn value_to_px_x_and_back_roundtrip() {
        let rect = test_rect();
        let bounds = (0.0, 400.0);
        for v in [0.0, 100.0, 250.0, 400.0] {
            let px = value_to_px_x(v, bounds, rect);
            let back = px_x_to_value(px, bounds, rect);
            assert!((back - v).abs() < 1e-2, "v={v} back={back}");
        }
        // Левый край холста — минимум диапазона, правый — максимум.
        assert!((value_to_px_x(0.0, bounds, rect) - rect.left()).abs() < 1e-4);
        assert!((value_to_px_x(400.0, bounds, rect) - rect.right()).abs() < 1e-4);
    }

    #[test]
    fn value_to_px_y_flips_because_screen_y_grows_down() {
        let rect = test_rect();
        let bounds = (0.0, 1.0);
        // Максимум значения (1.0) должен оказаться наверху холста (меньший Y),
        // минимум (0.0) — внизу (больший Y).
        assert!((value_to_px_y(1.0, bounds, rect) - rect.top()).abs() < 1e-4);
        assert!((value_to_px_y(0.0, bounds, rect) - rect.bottom()).abs() < 1e-4);
        for v in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let px = value_to_px_y(v, bounds, rect);
            let back = px_y_to_value(px, bounds, rect);
            assert!((back - v).abs() < 1e-2, "v={v} back={back}");
        }
    }

    #[test]
    fn curve_x_bounds_keeps_default_range_when_points_inside() {
        let curve = ResponseCurve::linear(10.0, 20.0);
        assert_eq!(curve_x_bounds(&curve, (0.0, 100.0)), (0.0, 100.0));
    }

    #[test]
    fn curve_x_bounds_expands_for_out_of_range_points() {
        let curve = ResponseCurve {
            points: vec![
                CurvePoint { x: -50.0, y: 0.0 },
                CurvePoint { x: 500.0, y: 1.0 },
            ],
        };
        assert_eq!(curve_x_bounds(&curve, (0.0, 400.0)), (-50.0, 500.0));
    }

    #[test]
    fn curve_x_bounds_never_degenerates() {
        let curve = ResponseCurve {
            points: vec![CurvePoint { x: 5.0, y: 0.0 }],
        };
        let (lo, hi) = curve_x_bounds(&curve, (5.0, 5.0));
        assert!(hi > lo);
    }

    #[test]
    fn source_label_switches_by_language_and_appends_unit() {
        let def = sources::def(SourceId::FlightAirspeedKn);
        assert!(source_label(def, Lang::En).contains("kn"));
        assert!(source_label(def, Lang::Ru).contains("kn"));
        assert_ne!(source_label(def, Lang::En), source_label(def, Lang::Ru));
    }

    #[test]
    fn source_label_skips_empty_unit() {
        let def = sources::def(SourceId::FlightOnGround);
        assert!(!source_label(def, Lang::En).contains('('));
    }

    #[test]
    fn source_group_separates_flight_wt_and_lvar() {
        let flight = source_group(sources::def(SourceId::FlightAirspeedKn).games);
        let wt = source_group(sources::def(SourceId::WtIasKmh).games);
        let lvar = source_group(sources::def(SourceId::Lvar).games);
        assert_ne!(flight, wt);
        assert_ne!(flight, lvar);
        assert_ne!(wt, lvar);
    }

    #[test]
    fn default_source_for_game_picks_matching_pipeline() {
        assert_eq!(default_source_for_game(ActiveGame::Wt), SourceId::WtIasKmh);
        assert_eq!(
            default_source_for_game(ActiveGame::Msfs),
            SourceId::FlightAirspeedKn
        );
        assert_eq!(
            default_source_for_game(ActiveGame::Xplane),
            SourceId::FlightAirspeedKn
        );
    }

    #[test]
    fn default_trigger_for_kind_between_is_never_inverted() {
        let def = sources::def(SourceId::FlightBankDeg); // диапазон (-90.0, 90.0)
        match default_trigger_for_kind(TriggerKind::Between, def) {
            Trigger::Between { lo, hi } => assert!(lo <= hi),
            _ => panic!("expected Between"),
        }
    }

    // -------------------------------------------------------------
    // A4: уникальность имени
    // -------------------------------------------------------------

    #[test]
    fn unique_effect_name_returns_base_when_free() {
        let effects: Vec<CustomEffect> = Vec::new();
        assert_eq!(
            unique_effect_name("Airspeed (kn)", &effects),
            "Airspeed (kn)"
        );
    }

    #[test]
    fn unique_effect_name_appends_running_number_starting_at_2() {
        let mut a = new_effect("Airspeed (kn)".into(), SourceId::FlightAirspeedKn);
        a.name = "Airspeed (kn)".into();
        let effects = vec![a];
        assert_eq!(
            unique_effect_name("Airspeed (kn)", &effects),
            "Airspeed (kn) 2"
        );
    }

    #[test]
    fn unique_effect_name_skips_numbers_already_taken() {
        let mut a = new_effect("X".into(), SourceId::FlightAirspeedKn);
        a.name = "X".into();
        let mut b = new_effect("X 2".into(), SourceId::FlightAirspeedKn);
        b.name = "X 2".into();
        let effects = vec![a, b];
        assert_eq!(unique_effect_name("X", &effects), "X 3");
    }

    // -------------------------------------------------------------
    // Задача B: время записи <-> пиксель, поиск действующего кадра
    // -------------------------------------------------------------

    #[test]
    fn session_x_bounds_guards_against_zero_duration() {
        assert_eq!(session_x_bounds(0.0), (0.0, 1.0));
        assert_eq!(session_x_bounds(120.5), (0.0, 120.5));
    }

    fn flight_frame(t: f64) -> SessionFrame {
        SessionFrame {
            t,
            frame: TelemetryFrame::Flight(crate::types::FlightVars::default()),
        }
    }

    #[test]
    fn frame_at_relative_time_picks_last_frame_at_or_before_t() {
        let frames = vec![flight_frame(10.0), flight_frame(11.0), flight_frame(12.0)];
        let t0 = frames[0].t;
        // Ровно на кадре.
        assert_eq!(frame_at_relative_time(&frames, t0, 1.0).unwrap().t, 11.0);
        // Между кадрами — берём последний, что не позже запрошенного.
        assert_eq!(frame_at_relative_time(&frames, t0, 1.9).unwrap().t, 11.0);
        // До первого кадра — сваливаемся на первый, а не в None.
        assert_eq!(frame_at_relative_time(&frames, t0, -5.0).unwrap().t, 10.0);
        // После последнего — держим последний известный кадр.
        assert_eq!(frame_at_relative_time(&frames, t0, 999.0).unwrap().t, 12.0);
    }

    #[test]
    fn frame_at_relative_time_empty_is_none() {
        assert!(frame_at_relative_time(&[], 0.0, 1.0).is_none());
    }

    // -------------------------------------------------------------
    // "Запись сделана в другой игре" (график прогона записи, step_session)
    // -------------------------------------------------------------

    #[test]
    fn frame_is_other_game_detects_mismatch_by_telemetry_variant() {
        let wt_frame = TelemetryFrame::Wt(crate::wt_link::vars::WtVars::default());
        // Кадр Wt + источник Flight -> true (другая игра).
        let flight_source_games = sources::def(SourceId::FlightAirspeedKn).games;
        assert!(frame_is_other_game(&wt_frame, flight_source_games));

        // Кадр Wt + источник Wt -> false (та же игра).
        let wt_source_games = sources::def(SourceId::WtIasKmh).games;
        assert!(!frame_is_other_game(&wt_frame, wt_source_games));
    }

    #[test]
    fn game_mask_label_lists_only_enabled_games() {
        let mask = sources::def(SourceId::WtIasKmh).games;
        let t = &crate::i18n::EN;
        assert_eq!(game_mask_label(mask, t), t.hover_game_wt);

        let flight_mask = sources::def(SourceId::FlightAirspeedKn).games;
        assert_eq!(
            game_mask_label(flight_mask, t),
            format!("{}/{}", t.hover_game_msfs, t.hover_game_xp)
        );
    }

    #[test]
    fn telemetry_game_label_names_flight_pipeline_jointly() {
        let t = &crate::i18n::EN;
        let flight_frame = TelemetryFrame::Flight(crate::types::FlightVars::default());
        assert_eq!(
            telemetry_game_label(&flight_frame, t),
            format!("{}/{}", t.hover_game_msfs, t.hover_game_xp)
        );

        let wt_frame = TelemetryFrame::Wt(crate::wt_link::vars::WtVars::default());
        assert_eq!(telemetry_game_label(&wt_frame, t), t.hover_game_wt);
    }

    // -------------------------------------------------------------
    // Задача 1: заготовки нового эффекта
    // -------------------------------------------------------------

    /// Общая проверка для всех 4 заготовок сразу — требование задачи
    /// дословно: частота формы не выше `MAX_SHAPE_FREQ_HZ`, кривая непустая,
    /// хотя бы один мотор включён на выходе.
    #[test]
    fn all_presets_respect_freq_cap_have_curve_and_output() {
        let def = sources::def(SourceId::FlightAirspeedKn);
        for preset in EFFECT_PRESETS {
            let mut e = new_effect("T".into(), SourceId::FlightAirspeedKn);
            apply_preset(&mut e, preset, def);
            if let Some(f) = e.shape.freq_hz() {
                assert!(
                    f <= MAX_SHAPE_FREQ_HZ,
                    "{preset:?}: freq {f} exceeds MAX_SHAPE_FREQ_HZ"
                );
            }
            assert!(
                !e.curve.points.is_empty(),
                "{preset:?}: curve must not be empty"
            );
            assert!(
                e.out.joystick || e.out.throttle_left || e.out.throttle_right,
                "{preset:?}: at least one motor must be enabled"
            );
        }
    }

    #[test]
    fn preset_impact_is_a_one_shot_on_change() {
        let def = sources::def(SourceId::FlightAirspeedKn);
        let mut e = new_effect("T".into(), SourceId::FlightAirspeedKn);
        apply_preset(&mut e, EffectPreset::Impact, def);
        assert!(matches!(e.trigger, Trigger::Changed { .. }));
        assert!(matches!(e.shape, Shape::OneShot { .. }));
        // hold_s триггера обязан перекрывать всю огибающую OneShot — иначе
        // EMA (engine::step) обнулит хвост импульса раньше, чем он доиграет
        // decay (см. комментарий у apply_preset).
        if let (
            Trigger::Changed { hold_s, .. },
            Shape::OneShot {
                attack_ms,
                hold_ms,
                decay_ms,
                ..
            },
        ) = (e.trigger, e.shape)
        {
            let envelope_s = (attack_ms + hold_ms + decay_ms) as f64 / 1000.0;
            assert!(
                hold_s >= envelope_s,
                "trigger hold_s ({hold_s}) must cover the OneShot envelope ({envelope_s})"
            );
        }
    }

    #[test]
    fn preset_hum_and_pulsation_fire_above_threshold_with_hysteresis() {
        let def = sources::def(SourceId::FlightAirspeedKn);
        for preset in [EffectPreset::Hum, EffectPreset::Pulsation] {
            let mut e = new_effect("T".into(), SourceId::FlightAirspeedKn);
            apply_preset(&mut e, preset, def);
            match e.trigger {
                Trigger::Above { hysteresis, .. } => assert!(hysteresis > 0.0),
                _ => panic!("{preset:?}: expected Trigger::Above"),
            }
        }
        let mut hum = new_effect("T".into(), SourceId::FlightAirspeedKn);
        apply_preset(&mut hum, EffectPreset::Hum, def);
        assert!(matches!(hum.shape, Shape::Pulse { .. }));

        let mut pulsation = new_effect("T".into(), SourceId::FlightAirspeedKn);
        apply_preset(&mut pulsation, EffectPreset::Pulsation, def);
        assert!(matches!(pulsation.shape, Shape::Sine { .. }));
    }

    #[test]
    fn preset_growing_matches_always_on_default_behavior() {
        let def = sources::def(SourceId::FlightAirspeedKn);
        let mut e = new_effect("T".into(), SourceId::FlightAirspeedKn);
        apply_preset(&mut e, EffectPreset::Growing, def);
        assert_eq!(e.trigger, Trigger::Always);
        assert_eq!(e.shape, Shape::Constant);
        assert_eq!(
            e.curve,
            ResponseCurve::linear(def.default_range.0, def.default_range.1)
        );
    }

    // -------------------------------------------------------------
    // Зацикливание ручного предпросмотра событийных эффектов (Shape::OneShot
    // / Trigger::Changed): удар должен повторяться, а не играть один раз.
    // -------------------------------------------------------------

    fn impact_effect() -> CustomEffect {
        // Ровно заготовка "Удар" из задачи: attack 100ms + hold 200ms +
        // decay 190ms = 0.49s огибающая, Trigger::Changed { hold_s: 0.6 }.
        let mut e = new_effect("T".into(), SourceId::FlightAirspeedKn);
        e.trigger = Trigger::Changed {
            eps: 1.0,
            hold_s: 0.6,
        };
        e.shape = Shape::OneShot {
            attack_ms: 100.0,
            hold_ms: 200.0,
            decay_ms: 190.0,
            decay_exp: 2,
        };
        e
    }

    #[test]
    fn oneshot_preview_period_covers_full_envelope_plus_gap() {
        let e = impact_effect();
        // Огибающая OneShot 0.49s против hold_s 0.6s триггера — максимум 0.6s.
        let expected_period = 0.6 + PREVIEW_EVENT_REPEAT_GAP_S;
        assert!(is_event_effect(&e));
        assert!((event_duration_s(&e) - 0.6).abs() < 1e-9);

        // Чуть меньше периода — не завёрнуто.
        let t = expected_period - 0.05;
        assert!((preview_playback_time_s(&e, t) - t).abs() < 1e-9);

        // Ровно на границе периода — заворачивается в 0 (новый фронт).
        assert!(preview_playback_time_s(&e, expected_period).abs() < 1e-9);

        // Спустя несколько периодов — фаза внутри цикла та же, что в начале.
        let phase = 0.2;
        let wrapped = preview_playback_time_s(&e, expected_period * 3.0 + phase);
        assert!((wrapped - phase).abs() < 1e-6);
    }

    #[test]
    fn oneshot_preview_period_uses_longer_envelope_when_shape_dominates() {
        let mut e = impact_effect();
        // Огибающая OneShot длиннее hold_s триггера — период обязан покрыть
        // именно её, иначе следующий цикл оборвал бы decay на середине.
        e.shape = Shape::OneShot {
            attack_ms: 500.0,
            hold_ms: 500.0,
            decay_ms: 500.0,
            decay_exp: 1,
        };
        let expected_envelope = 1.5; // 0.5 + 0.5 + 0.5
        assert!((event_duration_s(&e) - expected_envelope).abs() < 1e-9);
    }

    #[test]
    fn continuous_effect_time_is_not_wrapped() {
        let mut e = new_effect("T".into(), SourceId::FlightAirspeedKn);
        e.trigger = Trigger::Always;
        e.shape = Shape::Sine {
            freq_hz: 2.0,
            depth_pct: 100.0,
        };
        assert!(!is_event_effect(&e));

        for t in [0.0, 1.5, 100.0, 999.25] {
            assert_eq!(
                preview_playback_time_s(&e, t),
                t,
                "непрерывный эффект не должен заворачиваться, иначе поедет фаза"
            );
        }
    }

    #[test]
    fn changed_trigger_alone_gives_period_no_shorter_than_hold_s() {
        let mut e = new_effect("T".into(), SourceId::FlightFlapsPct);
        e.shape = Shape::Constant; // без OneShot — событийность только от триггера
        e.trigger = Trigger::Changed {
            eps: 1.0,
            hold_s: 5.0,
        };
        assert!(is_event_effect(&e));
        assert!(event_duration_s(&e) >= 5.0);

        // Внутри hold_s — не завёрнуто (эффект ещё должен звучать).
        assert!((preview_playback_time_s(&e, 4.9) - 4.9).abs() < 1e-9);
        // Период обязан быть НЕ короче hold_s (плюс пауза) — иначе долгий
        // hold_s обрубился бы циклом раньше, чем сам успел отыграть.
        let period = event_duration_s(&e) + PREVIEW_EVENT_REPEAT_GAP_S;
        assert!(period >= 5.0);
        assert!((preview_playback_time_s(&e, period + 0.1) - 0.1).abs() < 1e-9);
    }

    // -------------------------------------------------------------
    // Задача 3 (custom LVAR source)
    // -------------------------------------------------------------

    #[test]
    fn read_effect_value_reads_lvar_by_name_from_flight_frame() {
        let mut fv = crate::types::FlightVars::default();
        fv.lvars.insert("L:A320_Gear_Nose".to_string(), 42.0);
        let frame = TelemetryFrame::Flight(fv);

        let mut e = new_effect("T".into(), SourceId::Lvar);
        e.lvar = Some(LvarSpec {
            name: "L:A320_Gear_Nose".to_string(),
            unit: "Number".to_string(),
        });
        assert_eq!(read_effect_value(&e, &frame), Some(42.0));
    }

    #[test]
    fn read_effect_value_is_none_for_unset_or_blank_lvar_name() {
        let frame = TelemetryFrame::Flight(crate::types::FlightVars::default());

        // Источник Lvar, но effect.lvar ещё None — пользователь только что
        // переключился, ничего не вписал.
        let e_none = new_effect("T".into(), SourceId::Lvar);
        assert_eq!(read_effect_value(&e_none, &frame), None);

        // effect.lvar есть, но имя — пустая/пробельная строка.
        let mut e_blank = new_effect("T".into(), SourceId::Lvar);
        e_blank.lvar = Some(LvarSpec {
            name: "   ".to_string(),
            unit: "Number".to_string(),
        });
        assert_eq!(read_effect_value(&e_blank, &frame), None);
    }

    #[test]
    fn read_effect_value_falls_back_to_sources_read_for_non_lvar_sources() {
        let fv = crate::types::FlightVars {
            airspeed_indicated: 250.0,
            ..Default::default()
        };
        let frame = TelemetryFrame::Flight(fv);
        let e = new_effect("T".into(), SourceId::FlightAirspeedKn);
        assert_eq!(read_effect_value(&e, &frame), Some(250.0));
    }

    #[test]
    fn lvar_name_missing_prefix_hint_flags_names_without_colon() {
        assert!(lvar_name_missing_prefix_hint("A320_Gear_Nose"));
        assert!(!lvar_name_missing_prefix_hint("L:A320_Gear_Nose"));
        // Пустое/пробельное имя — подсказка про префикс не нужна, там и так
        // видно "нет сигнала" (нечего подсказывать про префикс пустоты).
        assert!(!lvar_name_missing_prefix_hint(""));
        assert!(!lvar_name_missing_prefix_hint("   "));
    }

    #[test]
    fn coerce_lvar_game_mask_forces_msfs_only() {
        let mut games = GameMask {
            msfs: false,
            xplane: true,
            wt: true,
        };
        coerce_lvar_game_mask(&mut games);
        assert!(games.msfs);
        assert!(!games.xplane);
        assert!(!games.wt);
    }

    #[test]
    fn coerce_lvar_game_mask_is_idempotent_on_already_correct_mask() {
        let mut games = GameMask {
            msfs: true,
            xplane: false,
            wt: false,
        };
        coerce_lvar_game_mask(&mut games);
        assert_eq!(
            games,
            GameMask {
                msfs: true,
                xplane: false,
                wt: false,
            }
        );
    }
}
