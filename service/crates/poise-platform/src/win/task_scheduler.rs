//! 任务计划程序：按 XML 定义注册任务、删除整个任务文件夹。安装与卸载时使用。

use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::TaskScheduler::{
    ITaskFolder, ITaskService, TASK_CREATE_OR_UPDATE, TASK_ENUM_HIDDEN,
    TASK_LOGON_INTERACTIVE_TOKEN, TaskScheduler,
};
use windows::Win32::System::Variant::VARIANT;
use windows::core::BSTR;

use super::com;

/// 注册（或更新）`\{folder}\{name}`，文件夹不存在时自动创建。
pub fn register(folder: &str, name: &str, xml: &str) -> windows::core::Result<()> {
    let root = root_folder()?;
    // SAFETY: COM 调用，参数在调用期间有效。
    unsafe {
        root.RegisterTask(
            &BSTR::from(format!(r"\{folder}\{name}")),
            &BSTR::from(xml),
            TASK_CREATE_OR_UPDATE.0,
            &VARIANT::default(),
            &VARIANT::default(),
            TASK_LOGON_INTERACTIVE_TOKEN,
            &VARIANT::default(),
        )?;
    }
    Ok(())
}

/// 删除 `\{folder}` 里的所有任务（包括隐藏的）和文件夹本身。文件夹不存在时什么都不做。
pub fn delete_folder(folder: &str) -> windows::core::Result<()> {
    let root = root_folder()?;
    // SAFETY: COM 调用，参数在调用期间有效。
    unsafe {
        let Ok(target) = root.GetFolder(&BSTR::from(format!(r"\{folder}"))) else {
            return Ok(());
        };
        let tasks = target.GetTasks(TASK_ENUM_HIDDEN.0)?;
        let names: Vec<BSTR> = (1..=tasks.Count()?)
            .map(|i| tasks.get_Item(&VARIANT::from(i))?.Name())
            .collect::<windows::core::Result<_>>()?;
        for name in names {
            target.DeleteTask(&name, 0)?;
        }
        root.DeleteFolder(&BSTR::from(folder), 0)
    }
}

fn root_folder() -> windows::core::Result<ITaskFolder> {
    com::ensure_initialized();
    // SAFETY: COM 调用；连接本机、以当前身份。
    unsafe {
        let service: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)?;
        service.Connect(
            &VARIANT::default(),
            &VARIANT::default(),
            &VARIANT::default(),
            &VARIANT::default(),
        )?;
        service.GetFolder(&BSTR::from(r"\"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::user::UserContext;

    /// 在任务计划程序里建一个临时文件夹和任务，再整个删除。手动运行：
    /// cargo test -p poise-platform task_scheduler -- --ignored
    #[test]
    #[ignore]
    fn registers_and_deletes_a_folder() {
        const FOLDER: &str = "PoiseTaskTest";
        let sid = UserContext::current_process_sid().unwrap();
        let xml = include_str!("../../../../../app/Poise.Core/Settings/AutostartTask.xml")
            .replace("{UserSid}", &sid)
            .replace("{AgentPath}", r"C:\Windows\System32\cmd.exe");
        register(FOLDER, &format!("Agent {sid}"), &xml).unwrap();
        delete_folder(FOLDER).unwrap();
        // 已经不存在时什么都不做。
        delete_folder(FOLDER).unwrap();
    }
}
