//! 分镜（`scene.json`）：数据结构、校验（含数字防错与降级）、composition HTML 生成。

use serde::{Deserialize, Serialize};

use crate::podcast::SubtitleEntry;

/// 场景数量上限（长音频抽稀，避免全程动个不停）
pub const MAX_SCENES: usize = 24;
/// 单个场景最短时长（秒），过短的场景会与相邻场景合并
pub const MIN_SCENE_SECONDS: f64 = 4.0;

const COMPOSITION_TEMPLATE: &str = include_str!("templates/explainer.html");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenePlan {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub meta: SceneMeta,
    #[serde(default)]
    pub scenes: Vec<Scene>,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SceneMeta {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub total_duration: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataPoint {
    pub value: f64,
    #[serde(default)]
    pub label: String,
}

/// 场景类型。时间一律用字幕条目索引（`from_entry`/`to_entry`），由服务端换算为秒，
/// 避免 LLM 写错时间戳。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Scene {
    Cover {
        from_entry: usize,
        to_entry: usize,
        #[serde(default)]
        title: String,
        #[serde(default)]
        subtitle: String,
    },
    Chapter {
        from_entry: usize,
        to_entry: usize,
        #[serde(default)]
        title: String,
    },
    Bullets {
        from_entry: usize,
        to_entry: usize,
        #[serde(default)]
        heading: String,
        #[serde(default)]
        items: Vec<String>,
    },
    Metric {
        from_entry: usize,
        to_entry: usize,
        #[serde(default)]
        value: String,
        #[serde(default)]
        unit: String,
        #[serde(default)]
        label: String,
        #[serde(default)]
        note: String,
        #[serde(default)]
        source_quote: String,
    },
    Quote {
        from_entry: usize,
        to_entry: usize,
        #[serde(default)]
        text: String,
        #[serde(default)]
        speaker: String,
    },
    #[serde(rename = "bar_chart")]
    BarChart {
        from_entry: usize,
        to_entry: usize,
        #[serde(default)]
        heading: String,
        #[serde(default)]
        unit: String,
        #[serde(default)]
        data: Vec<DataPoint>,
        #[serde(default)]
        highlight: Option<String>,
        #[serde(default)]
        source_quote: String,
    },
    End {
        from_entry: usize,
        to_entry: usize,
        #[serde(default)]
        title: String,
        #[serde(default)]
        subtitle: String,
    },
}

impl Scene {
    pub fn kind(&self) -> &'static str {
        match self {
            Scene::Cover { .. } => "cover",
            Scene::Chapter { .. } => "chapter",
            Scene::Bullets { .. } => "bullets",
            Scene::Metric { .. } => "metric",
            Scene::Quote { .. } => "quote",
            Scene::BarChart { .. } => "bar_chart",
            Scene::End { .. } => "end",
        }
    }

    pub fn range(&self) -> (usize, usize) {
        match self {
            Scene::Cover { from_entry, to_entry, .. }
            | Scene::Chapter { from_entry, to_entry, .. }
            | Scene::Bullets { from_entry, to_entry, .. }
            | Scene::Metric { from_entry, to_entry, .. }
            | Scene::Quote { from_entry, to_entry, .. }
            | Scene::BarChart { from_entry, to_entry, .. }
            | Scene::End { from_entry, to_entry, .. } => (*from_entry, *to_entry),
        }
    }

    fn set_range(&mut self, from: usize, to: usize) {
        match self {
            Scene::Cover { from_entry, to_entry, .. }
            | Scene::Chapter { from_entry, to_entry, .. }
            | Scene::Bullets { from_entry, to_entry, .. }
            | Scene::Metric { from_entry, to_entry, .. }
            | Scene::Quote { from_entry, to_entry, .. }
            | Scene::BarChart { from_entry, to_entry, .. }
            | Scene::End { from_entry, to_entry, .. } => {
                *from_entry = from;
                *to_entry = to;
            }
        }
    }

    /// 该场景声明的数字（用于"数字必须来自原文"的校验）
    fn numbers(&self) -> Vec<String> {
        match self {
            Scene::Metric { value, .. } => number_tokens(value),
            Scene::BarChart { data, .. } => data
                .iter()
                .flat_map(|d| number_tokens(&format!("{}", d.value)))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// 降级为要点卡（数字无法核实、或场景类型内容不足时）
    fn degrade_to_bullets(&self) -> Option<Scene> {
        let (from_entry, to_entry) = self.range();
        let (heading, items) = match self {
            Scene::Metric { label, value, unit, note, .. } => {
                let head = if label.is_empty() { "关键数据".to_string() } else { label.clone() };
                let mut items = vec![format!("{value}{unit}")];
                if !note.is_empty() {
                    items.push(note.clone());
                }
                (head, items)
            }
            Scene::BarChart { heading, data, unit, .. } => {
                let items = data
                    .iter()
                    .map(|d| format!("{}：{}{}", d.label, d.value, unit))
                    .collect::<Vec<_>>();
                (heading.clone(), items)
            }
            _ => return None,
        };
        if items.is_empty() {
            return None;
        }
        Some(Scene::Bullets {
            from_entry,
            to_entry,
            heading,
            items,
        })
    }
}

/// 从 `subtitle.srt` 解析条目（序号从 0 开始，与 `from_entry`/`to_entry` 对应）
pub fn parse_srt(text: &str) -> Vec<SubtitleEntry> {
    let mut out = Vec::new();
    let mut start: Option<f64> = None;
    let mut end: Option<f64> = None;
    let mut body: Vec<String> = Vec::new();

    let flush = |start: &mut Option<f64>, end: &mut Option<f64>, body: &mut Vec<String>, out: &mut Vec<SubtitleEntry>| {
        if let (Some(s), Some(e)) = (start.take(), end.take()) {
            let text = body.join(" ").trim().to_string();
            if !text.is_empty() {
                out.push(SubtitleEntry { start: s, end: e, text });
            }
        }
        body.clear();
    };

    for line in text.lines() {
        let l = line.trim_end();
        if l.contains("-->") {
            flush(&mut start, &mut end, &mut body, &mut out);
            if let Some((a, b)) = l.split_once("-->") {
                start = parse_ts(a.trim());
                end = parse_ts(b.split_whitespace().next().unwrap_or(""));
            }
        } else if l.trim().is_empty() {
            flush(&mut start, &mut end, &mut body, &mut out);
        } else if l.chars().all(|c| c.is_ascii_digit()) {
            // 序号行，忽略
        } else {
            body.push(l.trim().to_string());
        }
    }
    flush(&mut start, &mut end, &mut body, &mut out);
    out
}

fn parse_ts(s: &str) -> Option<f64> {
    // 00:00:02,911
    let s = s.replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: f64 = parts[0].parse().ok()?;
    let m: f64 = parts[1].parse().ok()?;
    let sec: f64 = parts[2].parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

/// 提取字符串中的数字片段（支持小数）
pub fn number_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            cur.push(ch);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// 单位倍率（"10" + "亿" = 1e9）
fn unit_multiplier(unit: &str) -> f64 {
    match unit.trim() {
        "百" => 1e2,
        "千" => 1e3,
        "万" => 1e4,
        "亿" => 1e8,
        "万亿" => 1e12,
        _ => 1.0,
    }
}

/// 解析中文数字串（支持 十/百/千/万/亿 组合，如 十亿 → 1e9、一千二百万 → 1.2e7）
fn parse_cn_numeral(run: &str) -> Option<f64> {
    if run.is_empty() {
        return None;
    }
    let digit = |c: char| -> Option<f64> {
        Some(match c {
            '零' | '〇' => 0.0,
            '一' => 1.0,
            '二' | '两' => 2.0,
            '三' => 3.0,
            '四' => 4.0,
            '五' => 5.0,
            '六' => 6.0,
            '七' => 7.0,
            '八' => 8.0,
            '九' => 9.0,
            _ => return None,
        })
    };
    let mut total = 0.0_f64;
    let mut section = 0.0_f64;
    let mut cur: Option<f64> = None;
    let mut any = false;
    for ch in run.chars() {
        if let Some(d) = digit(ch) {
            cur = Some(d);
            any = true;
            continue;
        }
        any = true;
        match ch {
            '十' => {
                section += cur.unwrap_or(1.0) * 10.0;
                cur = None;
            }
            '百' => {
                section += cur.unwrap_or(1.0) * 100.0;
                cur = None;
            }
            '千' => {
                section += cur.unwrap_or(1.0) * 1000.0;
                cur = None;
            }
            '万' => {
                section = (section + cur.unwrap_or(0.0)) * 1e4;
                total += section;
                section = 0.0;
                cur = None;
            }
            '亿' => {
                section = (section + cur.unwrap_or(0.0)) * 1e8;
                total += section;
                section = 0.0;
                cur = None;
            }
            _ => return None,
        }
    }
    if !any {
        return None;
    }
    let v = total + section + cur.unwrap_or(0.0);
    if v > 0.0 {
        Some(v)
    } else {
        None
    }
}

/// 从文本中提取所有中文数字串并解析
fn chinese_numbers_in(text: &str) -> Vec<f64> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if matches!(ch, '零' | '〇' | '一' | '二' | '两' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '十' | '百' | '千' | '万' | '亿') {
            cur.push(ch);
        } else {
            if let Some(v) = parse_cn_numeral(&cur) {
                out.push(v);
            }
            cur.clear();
        }
    }
    if let Some(v) = parse_cn_numeral(&cur) {
        out.push(v);
    }
    out
}

/// 归一化文本：去掉空白与千分位逗号，便于数字比对
fn normalize_for_number_match(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != ',')
        .collect()
}

/// 校验并修正分镜计划：
/// - 索引裁剪到合法范围、按起始排序、保证从 0 开始连续覆盖
/// - 过短场景合并、数量上限截断
/// - 数字类场景校验数字是否来自原文，否则降级为要点卡
///
/// 返回告警信息（写进思考链路日志）。
pub fn sanitize(plan: &mut ScenePlan, entries: &[SubtitleEntry]) -> Vec<String> {
    let mut warnings = Vec::new();
    if entries.is_empty() {
        plan.scenes.clear();
        warnings.push("字幕为空，无法生成分镜".to_string());
        return warnings;
    }
    let last = entries.len() - 1;
    let total = entries[last].end;

    // 1. 裁剪索引
    for sc in plan.scenes.iter_mut() {
        let (mut f, mut t) = sc.range();
        f = f.min(last);
        t = t.min(last);
        if t < f {
            std::mem::swap(&mut f, &mut t);
        }
        sc.set_range(f, t);
    }

    // 2. 排序 + 去重（同一范围只保留第一个）
    plan.scenes.sort_by_key(|s| s.range().0);
    let mut dedup: Vec<Scene> = Vec::new();
    for sc in plan.scenes.drain(..) {
        if let Some(prev) = dedup.last() {
            if prev.range() == sc.range() {
                continue;
            }
        }
        dedup.push(sc);
    }
    plan.scenes = dedup;

    // 3. 保证从 0 开始
    if let Some(first) = plan.scenes.first_mut() {
        let (f, t) = first.range();
        if f > 0 {
            warnings.push(format!("首个场景未覆盖开头，已从条目 0 起扩展（原 {}）", f));
            first.set_range(0, t.max(f));
        }
    } else {
        // 没有场景：用一个 cover 兜底
        plan.scenes.push(Scene::Cover {
            from_entry: 0,
            to_entry: last,
            title: plan.meta.title.clone(),
            subtitle: String::new(),
        });
        warnings.push("LLM 未返回场景，已用封面场景兜底".to_string());
    }

    // 4. 覆盖连续性：上一场景结束处若早于下一场景开始，则把上一场景延伸过去
    let n = plan.scenes.len();
    for i in 0..n {
        let (_, to) = plan.scenes[i].range();
        if i + 1 < n {
            let (nf, _) = plan.scenes[i + 1].range();
            if to + 1 < nf {
                // 中间有空洞：把当前场景延伸到下一场景开始前一条
                let (f, _) = plan.scenes[i].range();
                plan.scenes[i].set_range(f, nf.saturating_sub(1));
            }
        }
    }

    // 5. 过短场景合并（仅合并相邻且同类型）
    let mut merged: Vec<Scene> = Vec::new();
    for sc in plan.scenes.drain(..) {
        let (f, t) = sc.range();
        let dur = entries[t].end - entries[f].start;
        if dur < MIN_SCENE_SECONDS {
            if let Some(prev) = merged.last_mut() {
                if prev.kind() == sc.kind() {
                    let (pf, _) = prev.range();
                    prev.set_range(pf, t);
                    continue;
                }
                // 不同类型：把过短场景并入前一个场景的时间范围（内容丢弃）
                let (pf, _) = prev.range();
                prev.set_range(pf, t);
                warnings.push(format!("场景过短（{dur:.1}s）已并入前一个场景"));
                continue;
            }
        }
        merged.push(sc);
    }
    plan.scenes = merged;

    // 6. 数量上限：均匀抽稀（保留首尾）
    if plan.scenes.len() > MAX_SCENES {
        let keep = MAX_SCENES;
        let step = plan.scenes.len() as f64 / keep as f64;
        let mut kept: Vec<Scene> = Vec::new();
        for i in 0..keep {
            let idx = ((i as f64) * step).floor() as usize;
            if idx < plan.scenes.len() {
                kept.push(plan.scenes[idx].clone());
            }
        }
        if let Some(last_scene) = plan.scenes.last().cloned() {
            kept.pop();
            kept.push(last_scene);
        }
        warnings.push(format!("场景数超过上限 {MAX_SCENES}，已抽稀至 {}", kept.len()));
        plan.scenes = kept;
    }

    // 7. 数字校验（数据类场景必须能从原文中找到数字）
    let mut fixed: Vec<Scene> = Vec::new();
    for sc in plan.scenes.drain(..) {
        let (f, t) = sc.range();
        let covered = normalize_for_number_match(
            &entries[f..=t].iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join(""),
        );
        let nums = sc.numbers();
        let unit = match &sc {
            Scene::Metric { unit, .. } => unit.clone(),
            Scene::BarChart { unit, .. } => unit.clone(),
            _ => String::new(),
        };
        let mult = unit_multiplier(&unit);
        let cn_values = chinese_numbers_in(&entries[f..=t].iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join(""));
        let unverified: Vec<String> = nums
            .iter()
            .filter(|n| {
                if n.is_empty() {
                    return false;
                }
                if covered.contains(n.as_str()) {
                    return false;
                }
                // 中文数字等价（如原文"十亿" vs 数据卡"10亿"）
                if let Ok(v) = n.parse::<f64>() {
                    let effective = v * mult;
                    if cn_values.iter().any(|c| (c - effective).abs() <= effective.abs() * 0.01 + 1e-9) {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();
        if !unverified.is_empty() {
            match sc.degrade_to_bullets() {
                Some(b) => {
                    warnings.push(format!(
                        "场景 {} 的数字 {:?} 未在原文中找到，已降级为要点卡",
                        sc.kind(),
                        unverified
                    ));
                    fixed.push(b);
                }
                None => {
                    warnings.push(format!("场景 {} 内容不足，已丢弃", sc.kind()));
                }
            }
        } else {
            fixed.push(sc);
        }
    }
    plan.scenes = fixed;

    // 8. 元信息
    plan.meta.total_duration = total;
    plan.meta.scene_count();
    if plan.style.is_empty() {
        plan.style = "dark-tech".to_string();
    }
    warnings
}

impl SceneMeta {
    fn scene_count(&self) {}
}

/// 已解析时间轴的场景（供 HTML 生成使用）
#[derive(Debug, Clone)]
pub struct ResolvedScene {
    #[allow(dead_code)]
    pub index: usize,
    #[allow(dead_code)]
    pub kind: String,
    /// 最终时间窗口（首段起点为 0，段间无空隙，末段延伸到音频结束）；测试与诊断使用
    #[allow(dead_code)]
    pub start: f64,
    #[allow(dead_code)]
    pub end: f64,
    pub html: String,
    pub data: serde_json::Value,
}

/// 把条目索引换算为秒。
///
/// **两遍处理**：先算出所有场景的最终时间窗口（首段从 0 开始、段间空隙由前一段填满、
/// 末段延伸到音频结束），再用最终窗口生成 HTML 与动画数据。
/// 若先生成 HTML 再填隙，会导致 HTML 片段窗口之间留下空档 → **黑屏**。
pub fn resolve(plan: &ScenePlan, entries: &[SubtitleEntry]) -> Vec<ResolvedScene> {
    let last = entries.len().saturating_sub(1);
    let total = entries.get(last).map(|e| e.end).unwrap_or(0.0);
    let n = plan.scenes.len();
    if n == 0 {
        return Vec::new();
    }

    // 第一遍：由字幕索引得到窗口骨架
    let mut wins: Vec<(f64, f64)> = Vec::with_capacity(n);
    for sc in &plan.scenes {
        let (f, t) = sc.range();
        let f = f.min(last);
        let t = t.min(last).max(f);
        wins.push((entries[f].start, entries[t].end));
    }

    // 首段从 0 开始
    if let Some(w) = wins.first_mut() {
        if w.0 > 0.0 {
            w.0 = 0.0;
        }
    }
    // 段间无空隙：前一段延伸到后一段开始；末段延伸到总时长
    for i in 0..n {
        let next_start = if i + 1 < n { wins[i + 1].0 } else { total };
        if wins[i].1 < next_start {
            wins[i].1 = next_start;
        }
        if wins[i].1 > next_start {
            wins[i].1 = next_start; // 防重叠
        }
        if wins[i].1 <= wins[i].0 {
            wins[i].1 = wins[i].0 + 0.1; // 兜底最短可见时长
        }
    }
    if let Some(w) = wins.last_mut() {
        if w.1 < total {
            w.1 = total;
        }
    }

    // 第二遍：用最终窗口生成 HTML 与动画数据
    plan.scenes
        .iter()
        .enumerate()
        .map(|(i, sc)| {
            let (start, end) = wins[i];
            ResolvedScene {
                index: i,
                kind: sc.kind().to_string(),
                start,
                end,
                html: scene_html(i, sc, start, end),
                data: scene_data(i, sc, start, end),
            }
        })
        .collect()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn scene_html(index: usize, sc: &Scene, start: f64, end: f64) -> String {
    let open = |class: &str| {
        format!(
            "<section id=\"s{index}\" class=\"scene clip {class}\" data-start=\"{start:.3}\" data-duration=\"{dur:.3}\" data-track-index=\"{ti}\">",
            index = index,
            class = class,
            start = start,
            dur = (end - start).max(0.1),
            ti = index + 1
        )
    };
    let close = "</section>";

    match sc {
        Scene::Cover { title, subtitle, .. } => format!(
            "{}<img src=\"./assets/cover.png\" alt=\"\" /><div class=\"veil\"></div>\
             <div class=\"bar\"></div><h1>{}</h1><div class=\"sub\">{}</div>{}",
            open("cover"),
            esc(title),
            esc(subtitle),
            close
        ),
        Scene::Chapter { title, .. } => format!(
            "{}<div class=\"idx\">CHAPTER</div><h2>{}</h2><div class=\"rule\"></div>{}",
            open("chapter"),
            esc(title),
            close
        ),
        Scene::Bullets { heading, items, .. } => {
            let lis = items
                .iter()
                .enumerate()
                .map(|(k, it)| {
                    format!(
                        "<li><span class=\"n\">{:02}</span><span>{}</span></li>",
                        k + 1,
                        esc(it)
                    )
                })
                .collect::<String>();
            format!(
                "{}<h2>{}</h2><ul>{}</ul>{}",
                open("bullets"),
                esc(heading),
                lis,
                close
            )
        }
        Scene::Metric { value, unit, label, note, .. } => format!(
            "{}<div class=\"ring2\"></div><div class=\"ring\"></div><div class=\"num\"><span class=\"val\">{}</span><span class=\"unit\">{}</span></div><div class=\"label\">{}</div><div class=\"note\">{}</div>{}",
            open("metric"),
            esc(value),
            esc(unit),
            esc(label),
            esc(note),
            close
        ),
        Scene::Quote { text, speaker, .. } => format!(
            "{}<div class=\"mark\">&ldquo;</div><blockquote>{}</blockquote><div class=\"who\">{}</div>{}",
            open("quote"),
            esc(text),
            esc(speaker),
            close
        ),
        Scene::BarChart { heading, unit, data, highlight, .. } => {
            let max = data.iter().map(|d| d.value).fold(0.0_f64, f64::max).max(1.0);
            let bars = data
                .iter()
                .map(|d| {
                    let h = (d.value / max * 100.0).clamp(2.0, 100.0);
                    let on = highlight
                        .as_deref()
                        .map(|hl| hl == d.label)
                        .unwrap_or(false);
                    format!(
                        "<div class=\"bar{}\"><span class=\"bv\">{}{}</span><i style=\"height:{:.1}%\"></i><span class=\"bl\">{}</span></div>",
                        if on { " on" } else { "" },
                        if d.value.fract() == 0.0 { format!("{}", d.value as i64) } else { format!("{}", d.value) },
                        esc(unit),
                        h,
                        esc(&d.label)
                    )
                })
                .collect::<String>();
            format!(
                "{}<h2>{}</h2><div class=\"unit\">{}</div><div class=\"bars\">{}</div>{}",
                open("chart"),
                esc(heading),
                esc(unit),
                bars,
                close
            )
        }
        Scene::End { title, subtitle, .. } => format!(
            "{}<h2>{}</h2><div class=\"sub\">{}</div>{}",
            open("end"),
            esc(title),
            esc(subtitle),
            close
        ),
    }
}

fn scene_data(index: usize, sc: &Scene, start: f64, end: f64) -> serde_json::Value {
    let mut v = serde_json::json!({
        "i": index,
        "type": sc.kind(),
        "start": start,
        "end": end,
    });
    if let Scene::Metric { value, .. } = sc {
        if let Ok(n) = value.replace(',', "").parse::<f64>() {
            let decimals = value.split('.').nth(1).map(|d| d.len()).unwrap_or(0);
            if let Some(obj) = v.as_object_mut() {
                obj.insert("value_num".into(), serde_json::json!(n));
                obj.insert("value_decimals".into(), serde_json::json!(decimals));
            }
        }
    }
    v
}

pub struct CompositionOpts {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub disclaimer: Option<String>,
}

/// 生成 HyperFrames composition HTML（模板 + 静态场景 DOM + 时间轴数据）
pub fn build_composition(
    plan: &ScenePlan,
    entries: &[SubtitleEntry],
    opts: &CompositionOpts,
) -> String {
    let resolved = resolve(plan, entries);
    let total = entries.last().map(|e| e.end).unwrap_or(1.0).max(0.1);

    let scenes_html = resolved.iter().map(|s| s.html.as_str()).collect::<String>();
    let scene_json = serde_json::to_string(
        &resolved.iter().map(|s| s.data.clone()).collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".to_string());

    let disclaimer = opts.disclaimer.clone().unwrap_or_default();

    COMPOSITION_TEMPLATE
        .replace("__WIDTH__", &opts.width.to_string())
        .replace("__HEIGHT__", &opts.height.to_string())
        .replace("__FPS__", &opts.fps.to_string())
        .replace("__DURATION__", &format!("{total:.3}"))
        .replace("__DISCLAIMER__", &esc(&disclaimer))
        .replace("__SCENES__", &scenes_html)
        .replace("__SCENE_DATA__", &scene_json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<SubtitleEntry> {
        vec![
            SubtitleEntry { start: 0.0, end: 3.0, text: "主持人：今天聊聊美联储降息".into() },
            SubtitleEntry { start: 3.0, end: 7.0, text: "第一种可能是金价冲高回落".into() },
            SubtitleEntry { start: 7.0, end: 12.0, text: "第二种可能是短线承压".into() },
            SubtitleEntry { start: 12.0, end: 18.0, text: "ChatGPT 每周活跃用户突破 10 亿".into() },
        ]
    }

    #[test]
    fn parse_srt_reads_entries() {
        let srt = "1\n00:00:00,000 --> 00:00:02,911\n主持人：你好\n\n2\n00:00:02,911 --> 00:00:05,822\n嘉宾：嗯\n\n";
        let e = parse_srt(srt);
        assert_eq!(e.len(), 2);
        assert!((e[0].end - 2.911).abs() < 1e-6);
        assert_eq!(e[0].text, "主持人：你好");
        assert_eq!(e[1].text, "嘉宾：嗯");
    }

    #[test]
    fn sanitize_degrades_bogus_number() {
        let e = entries();
        let mut plan = ScenePlan {
            version: 1,
            style: String::new(),
            meta: SceneMeta::default(),
            scenes: vec![
                Scene::Cover { from_entry: 0, to_entry: 0, title: "标题".into(), subtitle: String::new() },
                Scene::Metric {
                    from_entry: 3,
                    to_entry: 3,
                    value: "99".into(),
                    unit: "亿".into(),
                    label: "编造的数据".into(),
                    note: String::new(),
                    source_quote: String::new(),
                },
            ],
        };
        let w = sanitize(&mut plan, &e);
        assert!(w.iter().any(|s| s.contains("降级")), "warnings: {w:?}");
        assert_eq!(plan.scenes.iter().filter(|s| s.kind() == "metric").count(), 0);
    }

    #[test]
    fn sanitize_keeps_verified_number() {
        let e = entries();
        let mut plan = ScenePlan {
            version: 1,
            style: String::new(),
            meta: SceneMeta::default(),
            scenes: vec![Scene::Metric {
                from_entry: 3,
                to_entry: 3,
                value: "10".into(),
                unit: "亿".into(),
                label: "周活".into(),
                note: String::new(),
                source_quote: "ChatGPT 每周活跃用户突破 10 亿".into(),
            }],
        };
        let _ = sanitize(&mut plan, &e);
        assert_eq!(plan.scenes.len(), 1);
        assert_eq!(plan.scenes[0].kind(), "metric");
    }

    #[test]
    fn chinese_numbers_parse() {
        assert_eq!(parse_cn_numeral("十"), Some(10.0));
        assert_eq!(parse_cn_numeral("十亿"), Some(1e9));
        assert_eq!(parse_cn_numeral("一亿"), Some(1e8));
        assert_eq!(parse_cn_numeral("一千二百万"), Some(1.2e7));
        assert_eq!(parse_cn_numeral("三"), Some(3.0));
        assert_eq!(parse_cn_numeral(""), None);
        let vals = chinese_numbers_in("每周超过十亿人在使用，增长了三倍");
        assert!(vals.iter().any(|v| (*v - 1e9).abs() < 1.0));
        assert!(vals.iter().any(|v| (*v - 3.0).abs() < 1e-6));
    }

    #[test]
    fn metric_verified_via_chinese_numeral() {
        let mut e = entries();
        e[3].text = "ChatGPT 每周活跃用户突破十亿".into();
        let mut plan = ScenePlan {
            version: 1,
            style: String::new(),
            meta: SceneMeta::default(),
            scenes: vec![Scene::Metric {
                from_entry: 3,
                to_entry: 3,
                value: "10".into(),
                unit: "亿".into(),
                label: "周活".into(),
                note: String::new(),
                source_quote: String::new(),
            }],
        };
        let _ = sanitize(&mut plan, &e);
        // 原文用中文数字"十亿"，数据卡写"10亿" → 应通过校验而非降级
        assert_eq!(plan.scenes[0].kind(), "metric");
    }

    #[test]
    fn resolve_fills_gaps_and_covers_timeline() {
        let e = entries();
        let plan = ScenePlan {
            version: 1,
            style: "dark-tech".into(),
            meta: SceneMeta::default(),
            scenes: vec![
                Scene::Cover { from_entry: 0, to_entry: 0, title: "t".into(), subtitle: String::new() },
                Scene::Bullets { from_entry: 2, to_entry: 2, heading: "h".into(), items: vec!["a".into()] },
            ],
        };
        let r = resolve(&plan, &e);
        assert_eq!(r.len(), 2);
        assert!((r[0].start - 0.0).abs() < 1e-6);
        // 第一段被填满到第二段开始
        assert!((r[0].end - r[1].start).abs() < 1e-6);
        // 最后一段覆盖到音频结束
        assert!((r[1].end - e.last().unwrap().end).abs() < 1e-6);
    }

    #[test]
    fn composition_html_contains_scenes_and_duration() {
        let e = entries();
        let plan = ScenePlan {
            version: 1,
            style: "dark-tech".into(),
            meta: SceneMeta::default(),
            scenes: vec![
                Scene::Cover { from_entry: 0, to_entry: 1, title: "标题A".into(), subtitle: "副标题".into() },
                Scene::Metric { from_entry: 3, to_entry: 3, value: "10".into(), unit: "亿".into(), label: "周活".into(), note: String::new(), source_quote: String::new() },
            ],
        };
        let html = build_composition(&plan, &e, &CompositionOpts {
            width: 1920, height: 1080, fps: 24, disclaimer: Some("以上内容仅代表个人观点。".into()),
        });
        assert!(html.contains("data-composition-id=\"main\""));
        assert!(html.contains("data-duration=\"18.000\""));
        assert!(html.contains("标题A"));
        assert!(html.contains("id=\"s1\""));
        assert!(html.contains("以上内容仅代表个人观点。"));
        assert!(html.contains("value_num"));
    }
}

#[cfg(test)]
mod gap_tests {
    use super::*;

    fn entries_with_gaps() -> Vec<SubtitleEntry> {
        // 6 段字幕，场景只覆盖 [0]、[2]、[5] → 存在索引空隙 1、3-4
        vec![
            SubtitleEntry { start: 0.0, end: 4.0, text: "开场".into() },
            SubtitleEntry { start: 4.0, end: 8.0, text: "过渡 1".into() },
            SubtitleEntry { start: 8.0, end: 14.0, text: "要点".into() },
            SubtitleEntry { start: 14.0, end: 20.0, text: "过渡 2".into() },
            SubtitleEntry { start: 20.0, end: 26.0, text: "过渡 3".into() },
            SubtitleEntry { start: 26.0, end: 34.0, text: "结尾".into() },
        ]
    }

    /// 解析 HTML 中所有片段窗口 <(start, start+duration)>
    fn clip_windows(html: &str) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        for seg in html.split("<section").skip(1) {
            let get = |key: &str| -> Option<f64> {
                let i = seg.find(key)?;
                let rest = &seg[i + key.len()..];
                let v: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                v.parse().ok()
            };
            if let (Some(s), Some(d)) = (get("data-start=\""), get("data-duration=\"")) {
                out.push((s, s + d));
            }
        }
        out
    }

    #[test]
    fn no_black_gaps_between_scenes_html_windows() {
        let e = entries_with_gaps();
        let plan = ScenePlan {
            version: 1,
            style: "dark-tech".into(),
            meta: SceneMeta::default(),
            scenes: vec![
                Scene::Cover { from_entry: 0, to_entry: 0, title: "开场".into(), subtitle: String::new() },
                Scene::Bullets { from_entry: 2, to_entry: 2, heading: "要点".into(), items: vec!["甲".into()] },
                Scene::End { from_entry: 5, to_entry: 5, title: "结尾".into(), subtitle: String::new() },
            ],
        };
        let html = build_composition(
            &plan,
            &e,
            &CompositionOpts { width: 1080, height: 1920, fps: 24, disclaimer: None },
        );
        let w = clip_windows(&html);
        assert_eq!(w.len(), 3, "应生成 3 个场景片段");
        assert!((w[0].0 - 0.0).abs() < 1e-6, "首段应从 0 开始，实际 {}", w[0].0);
        for i in 0..w.len() - 1 {
            let gap = w[i + 1].0 - w[i].1;
            assert!(
                gap.abs() < 0.05,
                "片段 {i} 与 {} 之间存在 {gap:.1}s 空隙（会导致黑屏）: {:?} → {:?}",
                i + 1,
                w[i],
                w[i + 1]
            );
        }
        let total = e.last().unwrap().end;
        assert!((w.last().unwrap().1 - total).abs() < 0.05, "末段应延伸到音频结束");
    }

    #[test]
    fn resolve_windows_are_contiguous_and_cover_total() {
        let e = entries_with_gaps();
        let plan = ScenePlan {
            version: 1,
            style: String::new(),
            meta: SceneMeta::default(),
            scenes: vec![
                Scene::Cover { from_entry: 0, to_entry: 0, title: "a".into(), subtitle: String::new() },
                Scene::Quote { from_entry: 2, to_entry: 2, text: "b".into(), speaker: String::new() },
                Scene::End { from_entry: 5, to_entry: 5, title: "c".into(), subtitle: String::new() },
            ],
        };
        let r = resolve(&plan, &e);
        for i in 0..r.len() - 1 {
            assert!((r[i].end - r[i + 1].start).abs() < 1e-6, "第 {i} 段未衔接");
        }
        assert!((r.last().unwrap().end - e.last().unwrap().end).abs() < 1e-6);
    }
}

#[cfg(test)]
mod dump {
    /// 导出 composition 到 /tmp/gen 供诊断（hyperframes validate / inspect）
    #[test]
    fn dump_composition_for_debug() {
        let entries = vec![
            super::SubtitleEntry { start: 0.0, end: 6.0, text: "开场".into() },
            super::SubtitleEntry { start: 6.0, end: 20.0, text: "要点一".into() },
            super::SubtitleEntry { start: 20.0, end: 34.0, text: "要点二".into() },
            super::SubtitleEntry { start: 34.0, end: 48.0, text: "结尾".into() },
        ];
        let plan = super::ScenePlan {
            version: 1,
            style: "dark-tech".into(),
            meta: super::SceneMeta { title: "调试".into(), total_duration: 48.0 },
            scenes: vec![
                super::Scene::Cover { from_entry: 0, to_entry: 0, title: "调试标题".into(), subtitle: "副标题".into() },
                super::Scene::Bullets { from_entry: 1, to_entry: 1, heading: "三个要点".into(), items: vec!["甲".into(), "乙".into(), "丙".into()] },
                super::Scene::Metric { from_entry: 2, to_entry: 2, value: "10".into(), unit: "亿".into(), label: "用户规模".into(), note: String::new(), source_quote: String::new() },
                super::Scene::End { from_entry: 3, to_entry: 3, title: "完".into(), subtitle: String::new() },
            ],
        };
        let html = super::build_composition(
            &plan,
            &entries,
            &super::CompositionOpts { width: 1080, height: 1920, fps: 24, disclaimer: Some("免责声明".into()) },
        );
        std::fs::create_dir_all("/tmp/gen").unwrap();
        std::fs::write("/tmp/gen/index.html", html).unwrap();
        println!("已导出 /tmp/gen/index.html");
    }
}
