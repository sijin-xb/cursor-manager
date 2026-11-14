//! 光标管理器 —— 基于 GPUI Kit。
//!
//! 左侧列出已安装的光标主题，右侧预览并选择尺寸，
//! 应用时同步写入 default 继承 / gsettings / environment.d / niri / Hyprland / Xresources。

mod apply;
mod install;
mod online;
mod preview;
mod themes;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use gpui_kit::*;
use gpui_kit::RenderImage;
use gpui_kit::assets::Assets;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, Selectable, Sizable, Theme, ThemeMode, label::Label};

actions!(cursor_manager, [SelUp, SelDown, ApplySel, DelSel]);

/// 常用 XCursor 角色（形状清单面板用）
const STANDARD_ROLES: &[&str] = &[
    "left_ptr", "pointing_hand", "xterm", "watch", "left_ptr_watch", "crossed_circle",
    "crosshair", "fleur", "question_arrow", "center_ptr", "hand1", "hand2",
    "sb_h_double_arrow", "sb_v_double_arrow", "size_nwse", "size_nesw",
    "top_side", "bottom_side", "left_side", "right_side",
    "top_left_corner", "top_right_corner", "bottom_left_corner", "bottom_right_corner",
    "pencil", "grab", "grabbing", "zoom-in", "zoom-out", "progress",
];

const SIZES: [u32; 5] = [16, 24, 32, 48, 64];

static PROBE_ARMED: AtomicBool = AtomicBool::new(false);
static PROBE_MOVES: AtomicUsize = AtomicUsize::new(0);

/// 侧栏行首小图：每个主题的 left_ptr，名义 32，显示 min(实际宽, 30)。
fn compute_row_previews(themes: &[themes::CursorTheme]) -> Vec<Option<(Arc<RenderImage>, f32)>> {
    themes
        .iter()
        .map(|t| {
            themes::find_shape(t, &["left_ptr", "default", "arrow"])
                .and_then(|f| preview::parse_file(&f))
                .and_then(|imgs| preview::pick(&imgs, 32))
                .and_then(|x| {
                    let disp = (x.width as f32).min(30.);
                    preview::to_render_image(&x).map(|im| (im, disp))
                })
        })
        .collect()
}

struct CursorManagerApp {
    themes: Vec<themes::CursorTheme>,
    selected: Option<usize>,
    size: u32,
    /// 当前生效的主题（gsettings 值，-alt 孪生名已归一化），只在刷新/应用时更新
    current_id: Option<String>,
    snapshot: Vec<apply::TargetResult>,
    /// (渲染图, 显示边长)：显示边长 = min(实际像素, 64)
    previews: Vec<Option<preview::AnimShape>>,
    /// 各形状当前播放到的帧下标 / 帧时长累积预算（40ms tick 驱动）
    anim_idx: Vec<usize>,
    anim_budget_ms: Vec<u32>,
    /// 侧栏每行的 left_ptr 小图（与 themes 平行）
    row_previews: Vec<Option<(Arc<RenderImage>, f32)>>,
    preview_key: Option<(String, u32)>,
    sizes_avail: Vec<u32>,
    status: String,
    /// 卸载按钮的两段式确认状态
    pending_uninstall: bool,
    /// 本地列表排序：false = 名称，true = 修改时间（新→旧）
    sort_by_time: bool,
    /// M3 波浪滑杆状态：拖动中标志 / 当前波幅 / 相位 / 轨道几何
    wavy_drag: Rc<Cell<bool>>,
    wavy_amp: f32,
    wavy_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// false = 本地列表，true = 在线目录
    view_mode: bool,
    search: Option<Entity<InputState>>,
    /// 在线：选中目录项后的资产列表（或拉取失败原因）
    online_assets: Option<Result<online::ReleaseAssets, String>>,
    online_sel: Option<usize>,
    online_busy: bool,
    /// 在线子源：true = GNOME-Look（pling OCS），false = GitHub 策展
    pling_mode: bool,
    online_search: Option<Entity<InputState>>,
    pling_items: Option<Result<online::PlingPage, String>>,
    /// pling 缩略图解码缓存（id → 渲染图）
    thumb_cache: HashMap<u64, Option<Arc<RenderImage>>>,
    pling_page: u32,
    pling_query: Option<String>,
    pling_sort: &'static str,
    /// 已应用主题历史（新→旧），供撤销
    history: Vec<String>,
    show_hotspot: bool,
    /// 选中主题 cursors/ 下的文件名集合（形状清单用）
    shape_set: HashSet<String>,
}

impl CursorManagerApp {
    fn new() -> Self {
        let themes = themes::scan();
        let snap = apply::snapshot();
        let selected = snap
            .theme
            .as_ref()
            .and_then(|cur| themes.iter().position(|t| &t.id == cur || &t.name == cur));
        let row_previews = compute_row_previews(&themes);
        let mut app = Self {
            themes,
            selected,
            size: snap.size.unwrap_or(24),
            current_id: snap.theme,
            snapshot: snap.targets,
            previews: Vec::new(),
            anim_idx: Vec::new(),
            anim_budget_ms: Vec::new(),
            row_previews,
            preview_key: None,
            sizes_avail: Vec::new(),
            status: "就绪 —— 左侧选择主题，右下应用".into(),
            pending_uninstall: false,
            sort_by_time: false,
            wavy_drag: Rc::new(Cell::new(false)),
            wavy_amp: 0.0,
            wavy_bounds: Rc::new(Cell::new(None)),
            view_mode: false,
            search: None,
            online_assets: None,
            online_sel: None,
            online_busy: false,
            pling_mode: true,
            online_search: None,
            pling_items: None,
            thumb_cache: HashMap::new(),
            pling_page: 1,
            pling_query: None,
            pling_sort: "highRating",
            history: load_history(),
            show_hotspot: false,
            shape_set: HashSet::new(),
        };
        app.refresh_previews();
        app
    }

    /// 40ms 一拍的动画 ticker：只对含多帧的形状推进帧下标，
    /// 静态主题零重绘。窗口关闭（实体释放）后循环自动退出。
    pub fn start_anim(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(40))
                    .await;
                let mut animated = false;
                let alive = this
                    .update(cx, |s, cx| {
                        // M3 波浪：拖动时波幅增大；静止衰减到细波并停更（波形不滑动）
                        let target = if s.wavy_drag.get() { 3.2 } else { 1.1 };
                        s.wavy_amp += (target - s.wavy_amp) * 0.3;
                        let animating = (s.wavy_amp - target).abs() > 0.03;
                        for i in 0..s.previews.len() {
                            let Some(a) = &s.previews[i] else { continue };
                            if a.frames.len() < 2 {
                                continue;
                            }
                            animated = true;
                            let n = a.frames.len();
                            let delays = a.delays_ms.clone();
                            let mut budget = s.anim_budget_ms.get(i).copied().unwrap_or(0);
                            let mut idx = s.anim_idx.get(i).copied().unwrap_or(0) % n;
                            let mut guard = 0;
                            while budget >= delays[idx].max(10) && guard < n {
                                budget -= delays[idx].max(10);
                                idx = (idx + 1) % n;
                                guard += 1;
                            }
                            s.anim_budget_ms[i] = budget;
                            s.anim_idx[i] = idx;
                        }
                        if animated || animating {
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    fn refresh_previews(&mut self) {
        let Some(idx) = self.selected else {
            self.previews.clear();
            self.anim_idx.clear();
            self.anim_budget_ms.clear();
            self.sizes_avail.clear();
            self.preview_key = None;
            return;
        };
        let theme = self.themes[idx].clone();
        let key = (theme.id.clone(), self.size);
        if self.preview_key.as_ref() == Some(&key) {
            return;
        }
        self.previews = themes::PREVIEW_SHAPES
            .iter()
            .map(|(_, names)| {
                themes::find_shape(&theme, names)
                    .and_then(|f| preview::parse_file(&f))
                    .and_then(|imgs| preview::build_anim(imgs, self.size))
            })
            .collect();
        self.anim_idx = vec![0; self.previews.len()];
        self.anim_budget_ms = vec![0; self.previews.len()];
        self.shape_set = fs::read_dir(theme.dir.join("cursors"))
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        self.sizes_avail = themes::PREVIEW_SHAPES
            .iter()
            .find_map(|(_, names)| {
                themes::find_shape(&theme, names).and_then(|f| preview::parse_file(&f))
            })
            .map(|imgs| {
                let mut s: Vec<u32> = imgs.iter().map(|i| i.size).collect();
                s.sort_unstable();
                s.dedup();
                s
            })
            .unwrap_or_default();
        self.preview_key = Some(key);
    }

    /// 拖动中设置波浪滑杆值（frac ∈ [0,1] → 尺寸 8–128）
    fn set_wavy_frac(&mut self, frac: f32, cx: &mut Context<Self>) {
        let n = (8.0 + frac * 120.0).round().clamp(8.0, 128.0) as u32;
        if n != self.size {
            self.size = n;
            cx.notify();
        }
    }

    /// 松手：按最终尺寸重解码预览
    fn release_wavy(&mut self, cx: &mut Context<Self>) {
        self.preview_key = None;
        self.refresh_previews();
        cx.notify();
    }

    /// 统一的选中入口：行点击 / 键盘导航都走这里
    fn select_index(&mut self, i: usize, cx: &mut Context<Self>) {
        self.selected = Some(i);
        self.pending_uninstall = false;
        self.preview_key = None;
        self.refresh_previews();
        // 配置源状态随选中刷新，避免面板显示上一次应用的残留
        let snap = apply::snapshot();
        self.current_id = snap.theme;
        self.snapshot = snap.targets;
        cx.notify();
    }

    /// 应用主题并写入历史（供撤销）
    fn apply_theme(&mut self, id: &str, size: u32, cx: &mut Context<Self>) {
        let results = apply::apply(id, size);
        let failed: Vec<String> = results
            .iter()
            .filter(|r| !r.ok)
            .map(|r| format!("{}: {}", r.label, r.detail))
            .collect();
        self.status = if failed.is_empty() {
            format!("已应用到 {} 处配置", results.len())
        } else {
            format!("部分失败 —— {}", failed.join("；"))
        };
        self.history.retain(|t| t != id);
        self.history.insert(0, id.to_string());
        self.history.truncate(10);
        save_history(&self.history);
        let snap = apply::snapshot();
        self.current_id = snap.theme;
        self.snapshot = snap.targets;
        self.preview_key = None;
        self.refresh_previews();
        cx.notify();
    }

    /// 搜索框是否持有焦点（持有时不响应键盘导航）
    fn search_focused(&self, window: &Window, cx: &Context<Self>) -> bool {
        self.search
            .as_ref()
            .map(|e| Some(e.read(cx).focus_handle(cx)) == window.focused(cx))
            .unwrap_or(false)
    }

    /// 拉取 GNOME-Look 光标列表
    fn fetch_pling(&mut self, cx: &mut Context<Self>) {
        self.online_busy = true;
        cx.notify();
        let query = self.pling_query.clone();
        let page = self.pling_page.max(1);
        let sort = self.pling_sort;
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let r = online::pling_search(query.as_deref(), page, sort);
                    match r {
                        Ok(mut pg) => {
                            let cache = std::path::PathBuf::from(
                                std::env::var("HOME").unwrap_or_default(),
                            )
                            .join(".cache/cursor-manager/thumbs");
                            let _ = std::fs::create_dir_all(&cache);
                            let mut thumbs: HashMap<u64, Option<Arc<RenderImage>>> =
                                HashMap::new();
                            for it in &mut pg.items {
                                if it.thumb.is_empty() {
                                    continue;
                                }
                                let dest = cache.join(format!("{}.img", it.id));
                                if !dest.exists() {
                                    let _ = online::download_to(&it.thumb, &dest);
                                }
                                it.local_thumb =
                                    Some(dest.to_string_lossy().to_string());
                                thumbs.insert(it.id, preview::image_from_file(&dest));
                            }
                            (Ok(pg), thumbs)
                        }
                        Err(e) => (Err(e), HashMap::new()),
                    }
                })
                .await;
            let (page_res, thumbs) = res;
            let _ = this.update(cx, |s, cx| {
                s.online_busy = false;
                s.thumb_cache = thumbs;
                s.pling_items = Some(page_res);
                cx.notify();
            });
        })
        .detach();
    }

    /// 在线下载并安装（pling 直链 / GitHub 资产通用）
    fn download_and_install(&mut self, aname: String, url: String, cx: &mut Context<Self>) {
        self.online_busy = true;
        self.status = format!("正在下载 {aname} …");
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let tmp =
                        std::env::temp_dir().join(format!("cm-dl-{}", std::process::id()));
                    let _ = std::fs::remove_file(&tmp);
                    match online::download_to(&url, &tmp) {
                        Ok(bytes) => install::install(&tmp).map(|o| (o, bytes)),
                        Err(e) => Err(format!("下载失败: {e}")),
                    }
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.online_busy = false;
                match res {
                    Ok((o, bytes)) => {
                        s.status = format!("已下载 {}KB —— {}", bytes / 1024, o.detail);
                        s.themes = themes::scan();
                        s.row_previews = compute_row_previews(&s.themes);
                        if let Some(i) = s.themes.iter().position(|t| t.id == o.name) {
                            s.view_mode = false;
                            s.select_index(i, cx);
                        }
                    }
                    Err(e) => s.status = e,
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 渲染在线列表：GNOME-Look（缩略图/排序/翻页）与 GitHub 策展两级
    fn render_online(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = cx.theme();
        let fg = t.colors.foreground;
        let muted = t.colors.muted_foreground;
        let hoverbg = t.colors.secondary_hover;
        let errc = t.colors.danger;

        let mut list = div()
            .id("online-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pb_2();

        // 子源切换
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .pb_1()
                .child(
                    Button::new("src-pling")
                        .small()
                        .label("GNOME-Look")
                        .selected(self.pling_mode)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.pling_mode = true;
                            if this.pling_items.is_none() && !this.online_busy {
                                this.fetch_pling(cx);
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("src-github")
                        .small()
                        .label("GitHub 策展")
                        .selected(!self.pling_mode)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.pling_mode = false;
                            cx.notify();
                        })),
                ),
        );

        if self.online_busy {
            return list.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(muted)
                    .child("网络请求中…"),
            );
        }

        if self.pling_mode {
            // 搜索行（输入框在 render 主流程里创建，这里复用）
            if let Some(st) = &self.online_search {
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .pb_1()
                        .child(div().flex_1().child(Input::new(st)))
                        .child(
                            Button::new("pling-search")
                                .small()
                                .label("搜索")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.pling_query = this
                                        .online_search
                                        .as_ref()
                                        .map(|e| e.read(cx).value().to_string());
                                    this.pling_page = 1;
                                    this.fetch_pling(cx);
                                })),
                        )
                        .child(
                            Button::new("sort-rating")
                                .ghost()
                                .small()
                                .label("评分")
                                .selected(self.pling_sort == "highRating")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.pling_sort = "highRating";
                                    this.pling_page = 1;
                                    this.fetch_pling(cx);
                                })),
                        )
                        .child(
                            Button::new("sort-new")
                                .ghost()
                                .small()
                                .label("最新")
                                .selected(self.pling_sort == "new")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.pling_sort = "new";
                                    this.pling_page = 1;
                                    this.fetch_pling(cx);
                                })),
                        ),
                );
            }

            match &self.pling_items {
                Some(Err(e)) => {
                    list = list.child(
                        div().p_3().text_sm().text_color(errc).child(e.clone()),
                    );
                }
                Some(Ok(page)) => {
                    let pages = page.total.div_ceil(20).max(1);
                    list = list.child(
                        div()
                            .px_3()
                            .py_1()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("GNOME-Look 光标主题 共 {} 个", page.total)),
                    );
                    for (k, it) in page.items.iter().enumerate() {
                        let aname = it.name.clone();
                        let url = it.download.clone();
                        let row = div()
                            .id(SharedString::from(format!("pling-{}-{k}", it.id)))
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .mx_2()
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(move |s| s.bg(hoverbg))
                            .child(
                                div().w(px(56.)).h(px(36.)).overflow_hidden().children(
                                    match self.thumb_cache.get(&it.id) {
                                        Some(Some(im)) => Some(img(im.clone()).w(px(56.))),
                                        _ => None,
                                    },
                                ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .flex_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(fg)
                                            .child(it.name.clone()),
                                    )
                                    .child(div().text_xs().text_color(muted).child(format!(
                                        "{} · ★{} · {} 下载",
                                        it.author, it.score, it.downloads
                                    ))),
                            )
                            .child(if it.page_url.is_empty() {
                                div().into_any_element()
                            } else {
                                let page_url = it.page_url.clone();
                                Button::new(SharedString::from(format!(
                                    "pling-open-{}",
                                    it.id
                                )))
                                .ghost()
                                .small()
                                .label("↗")
                                .on_click(move |_, _, _| {
                                    let _ = std::process::Command::new("xdg-open")
                                        .arg(page_url.clone())
                                        .spawn();
                                })
                                .into_any_element()
                            });
                        list = list.child(row.on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.download_and_install(aname.clone(), url.clone(), cx);
                            },
                        )));
                    }
                    list = list.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_2()
                            .py_2()
                            .child(
                                Button::new("pling-prev")
                                    .ghost()
                                    .small()
                                    .label("‹ 上一页")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if this.pling_page > 1 {
                                            this.pling_page -= 1;
                                            this.fetch_pling(cx);
                                        }
                                    })),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(format!(
                                        "{}/{}",
                                        self.pling_page, pages
                                    )),
                            )
                            .child(
                                Button::new("pling-next")
                                    .ghost()
                                    .small()
                                    .label("下一页 ›")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let pages =
                                            this.pling_items.as_ref().map(|r| match r {
                                                Ok(p) => p.total.div_ceil(20).max(1),
                                                Err(_) => 1,
                                            }).unwrap_or(1);
                                        if this.pling_page < pages {
                                            this.pling_page += 1;
                                            this.fetch_pling(cx);
                                        }
                                    })),
                            ),
                    );
                }
                None => {}
            }
            return list;
        }

        // ── GitHub 策展（原两级列表） ──
        if let Some(_sel) = self.online_sel {
            list = list.child(
                div()
                    .id("back")
                    .flex()
                    .items_center()
                    .px_3()
                    .py_1p5()
                    .mx_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_sm()
                    .text_color(fg)
                    .hover(move |s| s.bg(hoverbg))
                    .child("‹ 返回目录")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.online_sel = None;
                        this.online_assets = None;
                        cx.notify();
                    })),
            );
        }

        match (&self.online_assets, self.online_sel) {
            (Some(Ok(assets)), Some(cat_idx)) => {
                let cat = &online::CATALOG[cat_idx];
                list = list.child(
                    div()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(muted)
                        .child(format!(
                            "{} · {} · {} 个文件（选 Linux 包）",
                            cat.name,
                            assets.tag,
                            assets.assets.len()
                        )),
                );
                for (k, (aname, url)) in assets.assets.iter().enumerate() {
                    if !online::is_installable_asset(aname) {
                        continue;
                    }
                    let aname = aname.clone();
                    let url = url.clone();
                    let label = aname.clone();
                    list = list.child(
                        div()
                            .id(SharedString::from(format!("asset-{k}")))
                            .flex()
                            .items_center()
                            .px_3()
                            .py_1p5()
                            .mx_2()
                            .rounded_sm()
                            .cursor_pointer()
                            .text_sm()
                            .text_color(fg)
                            .hover(move |s| s.bg(hoverbg))
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let url = url.clone();
                                this.download_and_install(aname.clone(), url, cx);
                            })),
                    );
                }
            }
            (Some(Err(e)), _) => {
                list = list.child(div().p_3().text_sm().text_color(errc).child(e.clone()));
            }
            _ => {
                list = list.child(
                    div()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(muted)
                        .child("选择一个来源，拉取它最新 release 的文件"),
                );
                for (k, cat) in online::CATALOG.iter().enumerate() {
                    let hover = hoverbg;
                    list = list.child(
                        div()
                            .id(SharedString::from(format!("cat-{k}")))
                            .flex()
                            .flex_col()
                            .px_3()
                            .py_1p5()
                            .mx_2()
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(move |s| s.bg(hover))
                            .child(div().text_sm().text_color(fg).child(cat.name.to_string()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(format!("{} · {}", cat.repo, cat.desc)),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.online_busy = true;
                                cx.notify();
                                let repo = cat.repo;
                                cx.spawn(async move |this, cx| {
                                    let res = cx
                                        .background_executor()
                                        .spawn(async move { online::latest_assets(repo) })
                                        .await;
                                    let _ = this.update(cx, |s, cx| {
                                        s.online_busy = false;
                                        s.online_sel = Some(k);
                                        s.online_assets = Some(res);
                                        cx.notify();
                                    });
                                })
                                .detach();
                            })),
                    );
                }
            }
        }
        list
    }
}

// ── 历史记录持久化 ──

fn history_path() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
        .join(".local/state/cursor-manager/history.json")
}

fn load_history() -> Vec<String> {
    fs::read_to_string(history_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_history(h: &[String]) {
    if let Some(p) = history_path().parent() {
        let _ = fs::create_dir_all(p);
    }
    if let Ok(s) = serde_json::to_string(h) {
        let _ = fs::write(history_path(), s);
    }
}

impl Render for CursorManagerApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let bg = t.colors.background;
        let fg = t.colors.foreground;
        let border = t.colors.border;
        let accent = t.colors.accent;
        let muted = t.colors.muted_foreground;
        let selbg = t.colors.secondary;
        let hoverbg = t.colors.secondary_hover;
        let okc = t.colors.success;

        let current_id = self.current_id.clone();

        // ── 左侧主题列表 ──
        let filter = self
            .search
            .as_ref()
            .map(|e| e.read(cx).value().to_lowercase())
            .unwrap_or_default();
        let mut order: Vec<usize> = (0..self.themes.len())
            .filter(|&i| {
                let t = &self.themes[i];
                filter.is_empty()
                    || t.name.to_lowercase().contains(&filter)
                    || t.id.to_lowercase().contains(&filter)
            })
            .collect();
        if self.sort_by_time {
            order.sort_by(|&a, &b| self.themes[b].mtime.cmp(&self.themes[a].mtime));
        } else {
            order.sort_by(|&a, &b| {
                self.themes[a]
                    .id
                    .to_lowercase()
                    .cmp(&self.themes[b].id.to_lowercase())
            });
        }
        let rows: Vec<_> = order
            .into_iter()
            .map(|i| {
                let theme = &self.themes[i];
                let is_sel = self.selected == Some(i);
                let is_cur = current_id.as_deref() == Some(theme.id.as_str());
                let row = div()
                    .id(SharedString::from(format!("theme-{i}")))
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .mx_2()
                    .rounded_xl() // M3 list item: medium-increased 圆角
                    .cursor_pointer()
                    .hover(move |s| s.bg(hoverbg))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            // 行首指针小图（M3 leading avatar）
                            .child(
                                div().w(px(34.)).flex().justify_center().children(
                                    self.row_previews.get(i).cloned().flatten().map(
                                        |(im, disp)| img(im).size(px(disp)),
                                    ),
                                ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(fg)
                                            .font_weight(if is_sel {
                                                FontWeight::SEMIBOLD
                                            } else {
                                                FontWeight::NORMAL
                                            })
                                            .child(theme.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(muted)
                                            .child(theme.id.clone()),
                                    ),
                            ),
                    )
                    .child(if is_cur {
                        div().size(px(6.)).rounded_full().bg(accent)
                    } else {
                        div()
                    });
                let row = if is_sel { row.bg(selbg) } else { row };
                row.on_click(cx.listener(move |this, _, _, cx| {
                    this.select_index(i, cx);
                }))
            })
            .collect();

        let search_row: AnyElement = if !self.view_mode {
            let st = self.search.get_or_insert_with(|| {
                cx.new(|cx| InputState::new(window, cx).placeholder("搜索主题…"))
            });
            div()
                .px_2()
                .pb_1()
                .w_full()
                .child(Input::new(st))
                .into_any_element()
        } else if self.pling_mode {
            let st = self.online_search.get_or_insert_with(|| {
                cx.new(|cx| InputState::new(window, cx).placeholder("搜索 GNOME-Look 光标…"))
            });
            div()
                .px_2()
                .pb_1()
                .w_full()
                .child(Input::new(st))
                .into_any_element()
        } else {
            div().into_any_element()
        };
        let _ = &border;

        let count_row: AnyElement = if !self.view_mode {
            let chip = |id: &'static str, label: &'static str, active: bool| {
                let base = div()
                    .id(SharedString::from(id))
                    .px_2()
                    .py_0p5()
                    .rounded_full() // M3 chip: full pill
                    .text_xs()
                    .cursor_pointer()
                    .child(label.to_string());
                if active {
                    base.bg(selbg).text_color(fg)
                } else {
                    base.border_1().border_color(border).text_color(muted)
                }
            };
            div()
                .flex()
                .items_center()
                .px_2()
                .pb_1()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(format!("已安装主题（{}）", self.themes.len())),
                )
                .child(div().flex_1())
                .child(
                    chip("sort-name", "名称", !self.sort_by_time).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.sort_by_time = false;
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    chip("sort-time", "时间", self.sort_by_time).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.sort_by_time = true;
                            cx.notify();
                        }),
                    ),
                )
                .into_any_element()
        } else {
            div().into_any_element()
        };

        let list_body: AnyElement = if self.view_mode {
            self.render_online(cx).into_any_element()
        } else {
            div()
                .id("theme-list")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .pb_2()
                .children(rows)
                .into_any_element()
        };

        let sidebar = div()
            .flex()
            .flex_col()
            .w(px(280.))
            .min_h_0()
            .border_r_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .child(
                        div()
                            .flex()
                            .rounded_full()
                            .border_1()
                            .border_color(border)
                            .p(px(2.))
                            .child(
                                div()
                                    .id("tab-local")
                                    .px_3()
                                    .py_0p5()
                                    .rounded_full()
                                    .text_xs()
                                    .cursor_pointer()
                                    .text_color(if !self.view_mode { fg } else { muted })
                                    .bg(if !self.view_mode { selbg } else { bg })
                                    .child("本地")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.view_mode = false;
                                        this.pending_uninstall = false;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("tab-online")
                                    .px_3()
                                    .py_0p5()
                                    .rounded_full()
                                    .text_xs()
                                    .cursor_pointer()
                                    .text_color(if self.view_mode { fg } else { muted })
                                    .bg(if self.view_mode { selbg } else { bg })
                                    .child("在线")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.view_mode = true;
                                        this.pending_uninstall = false;
                                        if this.pling_items.is_none()
                                            && !this.online_busy
                                            && this.pling_mode
                                        {
                                            this.fetch_pling(cx);
                                        }
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new("install")
                                    .ghost()
                                    .small()
                                    .label("安装…")
                                    .on_click(cx.listener(|_this, _, _, cx| {
                                        let rx = cx.prompt_for_paths(PathPromptOptions {
                                            files: true,
                                            directories: true,
                                            multiple: false,
                                            prompt: Some(
                                                "选择光标包：zip / 目录 / .cur / .ani / .icns".into(),
                                            ),
                                        });
                                        cx.spawn(async move |this, cx| {
                                            let picked = match rx.await {
                                                Ok(Ok(Some(v))) => v,
                                                _ => return,
                                            };
                                            let Some(src) = picked.into_iter().next() else { return };
                                            let outcome = install::install(&src);
                                            let _ = this.update(cx, |s, cx| {
                                                match outcome {
                                                    Ok(o) => {
                                                        s.status =
                                                            format!("已安装「{}」—— {}", o.name, o.detail);
                                                        s.themes = themes::scan();
                                                        s.row_previews =
                                                            compute_row_previews(&s.themes);
                                                        s.current_id = apply::snapshot().theme;
                                                        s.snapshot = apply::snapshot().targets;
                                                        if let Some(i) =
                                                            s.themes.iter().position(|t| t.id == o.name)
                                                        {
                                                            s.selected = Some(i);
                                                        }
                                                    }
                                                    Err(e) => s.status = format!("安装失败：{e}"),
                                                }
                                                s.preview_key = None;
                                                s.refresh_previews();
                                                cx.notify();
                                            });
                                        })
                                        .detach();
                                    })),
                            )
                            .child(
                                Button::new("refresh")
                                    .ghost()
                                    .small()
                                    .label("刷新")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.themes = themes::scan();
                                        this.row_previews =
                                            compute_row_previews(&this.themes);
                                        this.current_id = apply::snapshot().theme;
                                        this.snapshot = apply::snapshot().targets;
                                        this.preview_key = None;
                                        this.refresh_previews();
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(search_row)
            .child(count_row)
            .child(list_body);

        // ── 右侧内容 ──
        let content: AnyElement = match self.selected {
            None => div()
                .flex_1()
                .min_h_0()
                .flex()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child("从左侧选择一个光标主题")
                .into_any_element(),
            Some(idx) => {
                let theme = &self.themes[idx];

                let header = div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(div().text_lg().child(format!("{}（{}）", theme.name, theme.id)))
                    .child(
                        div().text_sm().text_color(muted).child(if theme.comment.is_empty() {
                            theme.dir.to_string_lossy().to_string()
                        } else {
                            format!("{} · {}", theme.comment, theme.dir.display())
                        }),
                    )
                    .child(
                        div().text_xs().text_color(muted).child(if self.sizes_avail.is_empty() {
                            "未读到内置尺寸".to_string()
                        } else {
                            format!(
                                "内置尺寸 {}",
                                self.sizes_avail
                                    .iter()
                                    .map(|s| s.to_string())
                                    .collect::<Vec<_>>()
                                    .join(" / ")
                            )
                        }),
                    );

                let grid = div()
                    .flex()
                    .flex_wrap()
                    .gap_4()
                    .p_3()
                    .rounded_2xl()
                    .border_1()
                    .border_color(border)
                    .children(self.previews.iter().enumerate().map(|(i, pv)| {
                        let (label, _) = themes::PREVIEW_SHAPES[i];
                        let inner: AnyElement = match pv {
                            Some(anim) if !anim.frames.is_empty() => {
                                let n = anim.frames.len();
                                let idx = self.anim_idx.get(i).copied().unwrap_or(0) % n;
                                let (im, disp, (xh, yh), nat) = &anim.frames[idx];
                                let scale = disp / *nat as f32;
                                let mut cell = div().relative().child(img(im.clone()).size(px(*disp)));
                                if self.show_hotspot {
                                    cell = cell.child(
                                        div()
                                            .absolute()
                                            .left(px(*xh as f32 * scale - 4.))
                                            .top(px(*yh as f32 * scale - 4.))
                                            .size(px(8.))
                                            .rounded_full()
                                            .border_1()
                                            .border_color(okc),
                                    );
                                }
                                cell.into_any_element()
                            }
                            _ => div().text_color(border).child("—").into_any_element(),
                        };
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_1()
                            .w(px(84.))
                            .child(
                                div()
                                    .h(px(68.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(inner),
                            )
                            .child(div().text_xs().text_color(muted).child(label))
                    }));

                let size_row = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div().flex().items_center().gap_2()
                        .child(Label::new("尺寸").text_sm())
                    .children(SIZES.map(|s| {
                        let mut b = Button::new(SharedString::from(format!("size-{s}")))
                            .small()
                            .label(format!("{s}"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.size = s;
                                this.preview_key = None;
                                this.refresh_previews();
                                cx.notify();
                            }));
                        if self.size == s {
                            b = b.selected(true);
                        } else {
                            // 从滑杆拖回某个预设时同步选中态
                        }
                        b
                    }))
                    .child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .flex_shrink_0()
                                .child(format!("{}px", self.size)),
                        )
                        .child(
                            Button::new("hotspot")
                                .ghost()
                                .small()
                                .label(if self.show_hotspot { "隐藏热点" } else { "显示热点" })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_hotspot = !this.show_hotspot;
                                    cx.notify();
                                })),
                        ),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_w(px(520.))
                            .h(px(30.))
                            .id("wavy-track")
                            .relative()
                            .child({
                                let tb = self.wavy_bounds.clone();
                                let weak = cx.weak_entity();
                                canvas(
                                    {
                                        let tb = tb.clone();
                                        move |bounds, _, _| {
                                            tb.set(Some(bounds));
                                            bounds
                                        }
                                    },
                                    {
                                        let weak = weak.clone();
                                        move |bounds, _, window, cx| {
                                            let Some(ent) = weak.upgrade() else { return };
                                            let app = ent.read(cx);
                                            let frac =
                                                ((app.size as f32 - 8.0) / 120.0)
                                                    .clamp(0.0, 1.0);
                                            let amp = app.wavy_amp;
                                            let mid =
                                                bounds.origin.y + bounds.size.height / 2.0;
                                            let thumb_x =
                                                bounds.left() + bounds.size.width * frac;
                                            let seg = |window: &mut gpui_kit::Window,
                                                       from: Pixels,
                                                       to: Pixels,
                                                       color: gpui_kit::Hsla| {
                                                let mut pb = PathBuilder::stroke(px(3.0));
                                                let w = f32::from(to - from);
                                                let steps =
                                                    ((w / 3.0).ceil() as usize).max(2);
                                                for k in 0..=steps {
                                                    let t = k as f32 / steps as f32;
                                                    let x = f32::from(from) + w * t;
                                                    let y = f32::from(mid)
                                                        + amp * (x / 9.0).sin();
                                                    let pt = point(px(x), px(y));
                                                    if k == 0 {
                                                        pb.move_to(pt);
                                                    } else {
                                                        pb.line_to(pt);
                                                    }
                                                }
                                                if let Ok(path) = pb.build() {
                                                    window.paint_path(path, color);
                                                }
                                            };
                                            seg(window, bounds.left(), thumb_x, fg);
                                            seg(window, thumb_x, bounds.right(), muted);
                                            let tbq = Bounds::new(
                                                point(thumb_x - px(5.5), mid - px(5.5)),
                                                size(px(11.), px(11.)),
                                            );
                                            let mut q = fill(tbq, fg);
                                            q.corner_radii = px(5.5).into();
                                            window.paint_quad(q);
                                        }
                                    },
                                )
                            })
                            .child({
                                let drag = self.wavy_drag.clone();
                                let tb = self.wavy_bounds.clone();
                                let weak = cx.weak_entity();
                                let frac_of = |b: Bounds<Pixels>, x: Pixels| -> f32 {
                                    ((x - b.left()).max(px(0.0)) / b.size.width)
                                        .clamp(0.0, 1.0)
                                };
                                div()
                                    .absolute()
                                    .inset_0()
                                    .cursor_pointer()
                                    .on_mouse_down(MouseButton::Left, {
                                        let drag = drag.clone();
                                        let tb = tb.clone();
                                        let weak = weak.clone();
                                        move |ev: &MouseDownEvent, _, cx| {
                                            drag.set(true);
                                            if let Some(b) = tb.get() {
                                                let f = frac_of(b, ev.position.x);
                                                let _ = weak.update(cx, |s, cx| {
                                                    s.set_wavy_frac(f, cx)
                                                });
                                            }
                                        }
                                    })
                                    .on_mouse_move({
                                        let drag = drag.clone();
                                        let tb = tb.clone();
                                        let weak = weak.clone();
                                        move |ev: &MouseMoveEvent, _, cx| {
                                            if drag.get() {
                                                if let Some(b) = tb.get() {
                                                    let f = frac_of(b, ev.position.x);
                                                    let _ = weak.update(cx, |s, cx| {
                                                        s.set_wavy_frac(f, cx)
                                                    });
                                                }
                                            }
                                        }
                                    })
                                    .on_mouse_up(MouseButton::Left, {
                                        let drag = drag.clone();
                                        let weak = weak.clone();
                                        move |_ev: &MouseUpEvent, _, cx| {
                                            if drag.get() {
                                                drag.set(false);
                                                let _ = weak.update(cx, |s, cx| {
                                                    s.release_wavy(cx)
                                                });
                                            }
                                        }
                                    })
                            })
                    );

                let targets = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .rounded_2xl()
                    .border_1()
                    .border_color(border)
                    .children(self.snapshot.iter().map(|r| {
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_sm()
                            .child(
                                div()
                                    .w(px(20.))
                                    .text_color(if r.ok { okc } else { muted })
                                    .child(if r.ok { "✓" } else { "—" }),
                            )
                            .child(div().w(px(120.)).child(r.label))
                            .child(div().flex_1().text_color(muted).child(r.detail.clone()))
                    }));

                let present = STANDARD_ROLES
                    .iter()
                    .filter(|r| self.shape_set.contains(**r))
                    .count();
                let roles = div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .items_center()
                    .p_2()
                    .rounded_2xl()
                    .border_1()
                    .border_color(border)
                    .child(
                        div()
                            .text_sm()
                            .w(px(96.))
                            .child(format!("角色 {}/{}", present, STANDARD_ROLES.len())),
                    )
                    .children(STANDARD_ROLES.iter().map(|r| {
                        let has = self.shape_set.contains(*r);
                        div()
                            .text_xs()
                            .px_1()
                            .rounded_sm()
                            .text_color(if has { okc } else { muted })
                            .child(if has {
                                format!("✓{}", r)
                            } else {
                                format!("✗{}", r)
                            })
                    }));

                let n_ok = self.snapshot.iter().filter(|r| r.ok).count();
                let uninstall_btn = if self.pending_uninstall {
                    Button::new("uninstall")
                        .danger()
                        .small()
                        .label("确认卸载？")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.pending_uninstall = false;
                            let Some(idx) = this.selected else { return };
                            let theme = this.themes[idx].clone();
                            if this.current_id.as_deref() == Some(theme.id.as_str()) {
                                this.status =
                                    format!("「{}」正在使用中，先切换到其它主题再卸载", theme.id);
                                cx.notify();
                                return;
                            }
                            match install::uninstall(&theme.dir) {
                                Ok(detail) => {
                                    this.status =
                                        format!("已卸载「{}」—— {}", theme.id, detail);
                                    this.themes = themes::scan();
                                    this.row_previews = compute_row_previews(&this.themes);
                                    this.selected = None;
                                    this.previews.clear();
                                    this.anim_idx.clear();
                                    this.anim_budget_ms.clear();
                                    this.preview_key = None;
                                    this.current_id = apply::snapshot().theme;
                                    this.snapshot = apply::snapshot().targets;
                                }
                                Err(e) => this.status = format!("卸载失败：{e}"),
                            }
                            cx.notify();
                        }))
                } else {
                    Button::new("uninstall")
                        .ghost()
                        .small()
                        .label("卸载")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.pending_uninstall = true;
                            cx.notify();
                        }))
                };
                let bottom = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(uninstall_btn)
                    .child(
                        Button::new("apply")
                            .primary()
                            .rounded(px(999.))
                            .label(format!("应用 {} / {}", theme.id, self.size))
                            .on_click(cx.listener(|this, _, _, cx| {
                                let Some(idx) = this.selected else { return };
                                let theme_id = this.themes[idx].id.clone();
                                let size = this.size;
                                this.apply_theme(&theme_id, size, cx);
                            })),
                    )
                    .child(if self.history.len() >= 2 {
                        Button::new("undo")
                            .ghost()
                            .small()
                            .label("撤销")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.history.len() < 2 {
                                    return;
                                }
                                let target = this.history[1].clone();
                                if !this.themes.iter().any(|t| t.id == target) {
                                    this.status = format!("「{}」已被卸载，无法撤销", target);
                                    cx.notify();
                                    return;
                                }
                                this.apply_theme(&target, this.size, cx);
                            }))
                            .into_any_element()
                    } else {
                        div().into_any_element()
                    })
                    .child(if install::is_user_theme(&theme.dir) {
                        Button::new("export")
                            .ghost()
                            .small()
                            .label("导出 zip")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let Some(idx) = this.selected else { return };
                                let dir = this.themes[idx].dir.clone();
                                match install::export_zip(&dir) {
                                    Ok(p) => this.status = format!("已导出到 {p}"),
                                    Err(e) => this.status = format!("导出失败：{e}"),
                                }
                                cx.notify();
                            }))
                            .into_any_element()
                    } else {
                        div().into_any_element()
                    })
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("配置源 {n_ok}/{} 就绪", self.snapshot.len())),
                    ))
                    .child(
                        div()
                            .w_full()
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(muted)
                                    .whitespace_nowrap()
                                    .child(self.status.clone()),
                            ),
                    );

                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .pt_3()
                    .child(header)
                    .child(grid)
                    .child(size_row)
                    .child(targets)
                    .child(roles)
                    .child(bottom)
                    .into_any_element()
            }
        };

        // 带诊断探针的根：CURSOR_MGR_DEBUG=1 时输出鼠标事件到 stderr
        let debug = std::env::var_os("CURSOR_MGR_DEBUG").is_some();
        let mut root = div()
            .id("app-root")
            .size_full()
            .flex()
            .flex_col()
            .bg(bg)
            .text_color(fg)
            .pt_2()
            .child(div().flex().flex_1().min_h_0().child(sidebar).child(content));

        // 键盘导航：↑↓ 选择、Enter 应用、Delete 进入卸载确认
        root = root
            .on_action(cx.listener(|this, _: &SelUp, window, cx| {
                if this.view_mode || this.search_focused(window, cx) {
                    return;
                }
                let n = this.themes.len();
                if n == 0 {
                    return;
                }
                let i = this.selected.map(|i| (i + n - 1) % n).unwrap_or(0);
                this.select_index(i, cx);
            }))
            .on_action(cx.listener(|this, _: &SelDown, window, cx| {
                if this.view_mode || this.search_focused(window, cx) {
                    return;
                }
                let n = this.themes.len();
                if n == 0 {
                    return;
                }
                let i = this.selected.map(|i| (i + 1) % n).unwrap_or(0);
                this.select_index(i, cx);
            }))
            .on_action(cx.listener(|this, _: &ApplySel, window, cx| {
                if this.view_mode || this.search_focused(window, cx) {
                    return;
                }
                let Some(idx) = this.selected else { return };
                let theme_id = this.themes[idx].id.clone();
                let size = this.size;
                this.apply_theme(&theme_id, size, cx);
            }))
            .on_action(cx.listener(|this, _: &DelSel, window, cx| {
                if this.view_mode || this.search_focused(window, cx) {
                    return;
                }
                if this.selected.is_some() {
                    this.pending_uninstall = true;
                    cx.notify();
                }
            }));

        if debug {
            // 元素级：命中测试若正常，任何点击都会冒泡到这里
            root = root.on_click(cx.listener(|_, ev: &ClickEvent, _, _| {
                let pos = match ev {
                    ClickEvent::Mouse(m) => m.up.position,
                    _ => return,
                };
                eprintln!("[probe] root on_click pos={:?}", pos);
            }));
            // 窗口级：on_mouse_event 只能在 paint 阶段注册，挂在 canvas 回调里
            root = root.child(canvas(
                |_, _, _| {},
                |_, _, window, _| {
                    if !PROBE_ARMED.swap(true, Ordering::Relaxed) {
                        eprintln!("[probe] armed（canvas 布局回调已运行）");
                    }
                    window.on_mouse_event(|ev: &MouseMoveEvent, _, _, _| {
                        let n = PROBE_MOVES.fetch_add(1, Ordering::Relaxed);
                        if n < 30 {
                            eprintln!("[probe] move #{n} pos={:?}", ev.position);
                        }
                    });
                    window.on_mouse_event(|ev: &MouseDownEvent, _, _, _| {
                        eprintln!("[probe] down pos={:?} button={:?}", ev.position, ev.button);
                    });
                    window.on_mouse_event(|ev: &MouseUpEvent, _, _, _| {
                        eprintln!("[probe] up   pos={:?}", ev.position);
                    });
                },
            ));
        }

        root
    }
}

/// 最小点击测试窗口：CURSOR_MGR_PROBE=1 时额外打开，用于把「我的应用结构」
/// 和「gpui/wayland 平台层」的问题隔离开。纯 gpui div，零组件、零标题栏。
struct ProbeView(bool);

impl Render for ProbeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("probe")
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(if self.0 { rgb(0x1f7a33) } else { rgb(0x7a1f2b) })
            .text_color(rgb(0xffffff))
            .text_lg()
            .child(if self.0 { "点击成功 ✓" } else { "点我" })
            .on_click(cx.listener(|this, _ev: &ClickEvent, _, cx| {
                eprintln!("[probe] minimal click fired");
                this.0 = true;
                cx.notify();
            }))
    }
}

fn main() {
    let app = gpui_kit::application().with_assets(Assets);
    app.run(|cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.bind_keys([
            KeyBinding::new("up", SelUp, None),
            KeyBinding::new("down", SelDown, None),
            KeyBinding::new("enter", ApplySel, None),
            KeyBinding::new("delete", DelSel, None),
        ]);

        // 探针窗口：先开，放在主窗口左边
        if std::env::var_os("CURSOR_MGR_PROBE").is_some() {
            let mut b = Bounds::centered(None, size(px(320.), px(220.)), cx);
            b.origin.x -= px(700.);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(b)),
                    ..Default::default()
                },
                cx,
                |_, cx| cx.new(|_| ProbeView(false)),
            )
            .expect("打开探针窗口失败");
        }

        let bounds = Bounds::centered(None, size(px(980.), px(680.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(820.), px(560.))),
            // 服务器端装饰：niri 画边框，不再用 CSD 标题栏
            window_decorations: Some(WindowDecorations::Server),
            // niri 的 hide-seat-from 按 app_id 匹配客户端，
            // 配合 config.kdl 里对该 app_id 隐藏 niri-agent 座位
            app_id: Some("cursor-manager".into()),
            ..Default::default()
        };
        let (_, view) =
            gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| CursorManagerApp::new()))
                .expect("打开窗口失败");
        view.update(cx, |v, cx| v.start_anim(cx));
        cx.activate(true);
    });
}
