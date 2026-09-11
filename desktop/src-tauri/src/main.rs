// AudioLink 桌面端入口
// release 下不弹控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    audiolink_desktop_lib::run()
}
