use director_core::Engine;
use serde_json::Value;
use std::{
    io::{self, Read},
    path::PathBuf,
};
fn main() {
    let mut args = std::env::args().skip(1);
    let home = PathBuf::from(args.next().unwrap_or_else(|| ".director-next".into()));
    let result = (|| {
        director_core::windows_runtime::verify_shared_directory(&home)?;
        let mut body = String::new();
        io::stdin()
            .read_to_string(&mut body)
            .map_err(|e| e.to_string())?;
        let request: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        let mode = args.next();
        if (mode.as_deref() == Some("--native-cli-acceptance") || home.join("native-cli-acceptance.json").try_exists().map_err(|e| e.to_string())?)
            && [
                "foundation.run",
                "images.design",
                "storyboard.run",
                "workflows.resume",
                "storyboard.decision.resolve",
                "images.produce",
                "units.regenerate",
                "foundation.choices.apply",
            ]
            .contains(&request["action"].as_str().unwrap_or(""))
        {
            return Err("HEADLESS_ASYNC_ACCEPTANCE_BLOCKED: 有界真实文本流程须由持续运行的桌面应用派发；一次性核心进程不会启动后立即退出".into());
        }
        let engine = match mode.as_deref() {
            None => Engine::new(home)?,
            Some("--native-cli-acceptance") => {
                let path = PathBuf::from(args.next().ok_or("缺少验收授权文件绝对路径")?);
                if args.next().is_some() {
                    return Err("不支持的额外启动参数".into());
                }
                let grant = director_core::native_acceptance::AcceptanceGrant::from_file(&path)?;
                Engine::with_native_acceptance(home, grant)?
            }
            _ => return Err("未知启动选项；默认仅离线".into()),
        };
        engine.dispatch(request)
    })();
    match result {
        Ok(v) => println!("{}", v),
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1)
        }
    }
}
