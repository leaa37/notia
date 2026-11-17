// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "android")]
    {
        let filter = android_logger::FilterBuilder::new()
            .filter_level(log::LevelFilter::Error)
            .filter_module("notia_telegram_ai", log::LevelFilter::Info)
            .filter_module("notia_telegram", log::LevelFilter::Info)
            .build();
        android_logger::init_once(
            android_logger::Config::default()
                .with_tag("notia")
                .with_max_level(log::LevelFilter::Info)
                .with_filter(filter),
        );
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = env_logger::builder()
            .filter_level(log::LevelFilter::Error)
            .filter_module("notia_telegram_ai", log::LevelFilter::Info)
            // One line per chat turn (model, areas, tools), without content:
            // what is needed to tell why an answer went wrong.
            .filter_module("notia_turn", log::LevelFilter::Info)
            // The life of the Telegram bot: why it does not run, its stops,
            // starts and recoveries, and Telegram failures.
            .filter_module("notia_telegram", log::LevelFilter::Info)
            .try_init();
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    if notia_app::server::headless::requested(std::env::args()) {
        std::process::exit(notia_app::server::headless::run(std::env::args().skip(1).collect()));
    }

    #[cfg(feature = "app")]
    notia_lib::run();

    #[cfg(not(feature = "app"))]
    {
        eprintln!("Esta compilación de Notia solo incluye el modo servidor: ejecutala con --headless.");
        std::process::exit(2);
    }
}
