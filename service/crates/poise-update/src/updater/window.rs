//! 进度窗口：系统的任务对话框（跟随系统字体、缩放和主题），带 Poise 图标和进度条。
//! 显示的内容由 [`super::view`] 决定，这里只负责对话框本身。
//!
//! - 第一段：下载（显示已下载 / 总大小），可以取消。
//! - 第二段：安装并启动，进度条从头再走一遍；开始后不能取消。
//! - 结束：成功时显示新版本号，稍后自动关闭；失败时进度条变红并说明原因，用户点"关闭"。
//!
//! 窗口每 200 毫秒（对话框的定时通知）读取一次共享状态，只在内容变化时更新。

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, S_FALSE, S_OK, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    PBST_ERROR, TASKDIALOG_COMMON_BUTTON_FLAGS, TASKDIALOG_FLAGS, TASKDIALOG_MESSAGES,
    TASKDIALOG_NOTIFICATIONS, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TDCBF_CANCEL_BUTTON,
    TDCBF_CLOSE_BUTTON, TDE_CONTENT, TDE_MAIN_INSTRUCTION, TDF_ALLOW_DIALOG_CANCELLATION,
    TDF_CALLBACK_TIMER, TDF_SHOW_PROGRESS_BAR, TDM_CLICK_BUTTON, TDM_ENABLE_BUTTON,
    TDM_NAVIGATE_PAGE, TDM_SET_ELEMENT_TEXT, TDM_SET_PROGRESS_BAR_POS, TDM_SET_PROGRESS_BAR_RANGE,
    TDM_SET_PROGRESS_BAR_STATE, TDN_BUTTON_CLICKED, TDN_CREATED, TDN_NAVIGATED, TDN_TIMER,
    TaskDialogIndirect,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, ICON_BIG, ICON_SMALL, IDCANCEL, IDCLOSE, IMAGE_ICON, LR_DEFAULTCOLOR,
    LoadImageW, SM_CXICON, SM_CXSMICON, SendMessageW, WM_SETICON,
};
use windows::core::{HRESULT, PCWSTR};

use super::progress::{Failure, Outcome, Shared, Stage};
use super::text::Text;
use super::view::{Bar, ending, view};

/// 进度条的刻度。
const RANGE: i32 = 1000;
/// 成功后停留多久自动关闭。
const AUTO_CLOSE: Duration = Duration::from_millis(2500);
/// 窗口宽度（对话框单位）：固定宽度，文字变化时窗口不左右跳动。
const WIDTH: u32 = 240;
/// 资源里的图标编号（见 resources/updater.rc）。
const ICON_ID: usize = 1;

/// 显示进度窗口，直到更新结束且用户关闭（或自动关闭），或者用户取消下载。
pub fn show(text: &Text, shared: &Shared) {
    // SAFETY: 取本程序自己的模块句柄。
    let instance = unsafe { GetModuleHandleW(None) }
        .map(|m| HINSTANCE(m.0))
        .unwrap_or_default();
    let mut dialog = Dialog {
        text,
        shared,
        instance,
        title: wide(&text.get("updater.title")),
        shown: None,
        part: 0,
        position: 0,
        locked: false,
        finished: None,
        ending: None,
    };
    let first = view(text, None, &Stage::Checking);
    let data = &raw mut dialog as isize;
    let page = dialog.page(
        &first.instruction,
        &first.content,
        TDF_SHOW_PROGRESS_BAR,
        TDCBF_CANCEL_BUTTON,
        data,
    );
    // SAFETY: 配置与其中的字符串在对话框关闭前都有效；回调里通过 data 访问 dialog，
    // 此期间本函数不再使用 dialog。
    if let Err(e) = unsafe { TaskDialogIndirect(&page.config, None, None, None) } {
        log::warn!("无法显示进度窗口：{e}");
    }
}

/// 一个页面的配置，连同它引用的字符串。
struct Page {
    config: TASKDIALOGCONFIG,
    _strings: [Vec<u16>; 2],
}

struct Dialog<'a> {
    text: &'a Text,
    shared: &'a Shared,
    instance: HINSTANCE,
    title: Vec<u16>,
    /// 当前显示的标题与说明。
    shown: Option<(String, String)>,
    /// 当前是第几段进度、进度条的位置（同一段里只前进不后退）。
    part: u8,
    position: i32,
    /// 安装开始后"取消"按钮已禁用。
    locked: bool,
    /// 结束页面显示的时刻，以及是否自动关闭。
    finished: Option<(Instant, bool)>,
    ending: Option<(Box<Page>, Bar)>,
}

impl Dialog<'_> {
    fn page(
        &self,
        instruction: &str,
        content: &str,
        flags: TASKDIALOG_FLAGS,
        buttons: TASKDIALOG_COMMON_BUTTON_FLAGS,
        data: isize,
    ) -> Box<Page> {
        let strings = [wide(instruction), wide(content)];
        let config = TASKDIALOGCONFIG {
            cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
            hInstance: self.instance,
            dwFlags: flags | TDF_CALLBACK_TIMER | TDF_ALLOW_DIALOG_CANCELLATION,
            dwCommonButtons: buttons,
            pszWindowTitle: PCWSTR(self.title.as_ptr()),
            Anonymous1: TASKDIALOGCONFIG_0 {
                pszMainIcon: PCWSTR(ICON_ID as *const u16),
            },
            pszMainInstruction: PCWSTR(strings[0].as_ptr()),
            pszContent: PCWSTR(strings[1].as_ptr()),
            pfCallback: Some(callback),
            lpCallbackData: data,
            cxWidth: WIDTH,
            ..Default::default()
        };
        Box::new(Page {
            config,
            _strings: strings,
        })
    }

    fn created(&mut self, hwnd: HWND) {
        self.shared.set_owner(hwnd.0 as isize);
        self.set_icons(hwnd);
        send(hwnd, TDM_SET_PROGRESS_BAR_RANGE, 0, (RANGE as isize) << 16);
        self.tick(hwnd);
    }

    fn tick(&mut self, hwnd: HWND) {
        if let Some((at, auto_close)) = self.finished {
            if auto_close && at.elapsed() >= AUTO_CLOSE {
                send(hwnd, TDM_CLICK_BUTTON, IDCLOSE.0 as usize, 0);
            }
            return;
        }
        let (version, stage) = self.shared.snapshot();
        if let Stage::Finished(outcome) = &stage {
            self.finish(hwnd, version.as_deref(), outcome);
            return;
        }
        let now = view(self.text, version.as_deref(), &stage);
        let texts = (now.instruction, now.content);
        if self.shown.as_ref() != Some(&texts) {
            set_text(hwnd, TDE_MAIN_INSTRUCTION.0, &texts.0);
            set_text(hwnd, TDE_CONTENT.0, &texts.1);
            self.shown = Some(texts);
        }
        if stage.part() != self.part {
            self.part = stage.part();
            self.position = 0;
            send(hwnd, TDM_SET_PROGRESS_BAR_POS, 0, 0);
        }
        let position = (now.fraction * RANGE as f32) as i32;
        if position > self.position {
            self.position = position;
            send(hwnd, TDM_SET_PROGRESS_BAR_POS, position as usize, 0);
        }
        if !stage.cancellable() && !self.locked {
            self.locked = true;
            send(hwnd, TDM_ENABLE_BUTTON, IDCANCEL.0 as usize, 0);
        }
    }

    fn finish(&mut self, hwnd: HWND, version: Option<&str>, outcome: &Outcome) {
        if *outcome == Outcome::Failed(Failure::Cancelled) {
            send(hwnd, TDM_CLICK_BUTTON, IDCANCEL.0 as usize, 0);
            return;
        }
        let end = ending(self.text, version, outcome);
        let flags = if end.bar == Bar::None {
            TASKDIALOG_FLAGS(0)
        } else {
            TDF_SHOW_PROGRESS_BAR
        };
        let data = self as *mut Self as isize;
        let page = self.page(
            &end.instruction,
            &end.content,
            flags,
            TDCBF_CLOSE_BUTTON,
            data,
        );
        self.finished = Some((Instant::now(), end.auto_close));
        let config = &raw const page.config;
        self.ending = Some((page, end.bar));
        send(hwnd, TDM_NAVIGATE_PAGE, 0, config as isize);
    }

    fn navigated(&mut self, hwnd: HWND) {
        let Some((_, bar)) = &self.ending else {
            return;
        };
        if *bar != Bar::None {
            send(hwnd, TDM_SET_PROGRESS_BAR_RANGE, 0, (RANGE as isize) << 16);
            send(hwnd, TDM_SET_PROGRESS_BAR_POS, RANGE as usize, 0);
        }
        if *bar == Bar::Error {
            send(hwnd, TDM_SET_PROGRESS_BAR_STATE, PBST_ERROR as usize, 0);
        }
    }

    /// 返回 S_OK 关闭窗口，S_FALSE 保持打开。
    fn clicked(&self, button: i32) -> HRESULT {
        if self.finished.is_some() || button != IDCANCEL.0 {
            return S_OK;
        }
        let (_, stage) = self.shared.snapshot();
        if stage.cancellable() {
            log::info!("用户取消了更新");
            self.shared.cancel();
            S_OK
        } else if matches!(stage, Stage::Finished(_)) {
            S_OK
        } else {
            S_FALSE
        }
    }

    fn set_icons(&self, hwnd: HWND) {
        for (which, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
            // SAFETY: 从本程序的资源加载图标，尺寸取系统设置。
            let size = unsafe { GetSystemMetrics(metric) };
            let icon = unsafe {
                LoadImageW(
                    Some(self.instance),
                    PCWSTR(ICON_ID as *const u16),
                    IMAGE_ICON,
                    size,
                    size,
                    LR_DEFAULTCOLOR,
                )
            };
            if let Ok(icon) = icon {
                send_raw(hwnd, WM_SETICON, which as usize, icon.0 as isize);
            }
        }
    }
}

unsafe extern "system" fn callback(
    hwnd: HWND,
    msg: TASKDIALOG_NOTIFICATIONS,
    wparam: WPARAM,
    _lparam: LPARAM,
    data: isize,
) -> HRESULT {
    // SAFETY: data 是 show() 里传入的 Dialog，对话框关闭前一直有效，且只在界面线程上访问。
    let dialog = unsafe { &mut *(data as *mut Dialog) };
    match msg {
        TDN_CREATED => dialog.created(hwnd),
        TDN_NAVIGATED => dialog.navigated(hwnd),
        TDN_TIMER => dialog.tick(hwnd),
        TDN_BUTTON_CLICKED => return dialog.clicked(wparam.0 as i32),
        _ => {}
    }
    S_OK
}

fn set_text(hwnd: HWND, element: i32, text: &str) {
    let w = wide(text);
    send(
        hwnd,
        TDM_SET_ELEMENT_TEXT,
        element as usize,
        w.as_ptr() as isize,
    );
}

fn send(hwnd: HWND, msg: TASKDIALOG_MESSAGES, wparam: usize, lparam: isize) {
    send_raw(hwnd, msg.0 as u32, wparam, lparam);
}

fn send_raw(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) {
    // SAFETY: 发给本线程的对话框；指针参数在调用期间有效（对话框会复制字符串）。
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))) };
}

/// 以 NUL 结尾的 UTF-16。
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
