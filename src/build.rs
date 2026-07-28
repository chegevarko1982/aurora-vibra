fn main() {
    #[cfg(windows)]
    {
        use std::path::PathBuf;

        println!("cargo:rerun-if-changed=windows/resource.rc.in");
        println!("cargo:rerun-if-changed=assets/icon.ico");

        let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

        let major = std::env::var("CARGO_PKG_VERSION_MAJOR").unwrap();
        let minor = std::env::var("CARGO_PKG_VERSION_MINOR").unwrap();
        let patch = std::env::var("CARGO_PKG_VERSION_PATCH").unwrap();
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        // CARGO_PKG_AUTHORS разделяет несколько авторов через ':'.
        let company = std::env::var("CARGO_PKG_AUTHORS")
            .unwrap_or_default()
            .replace(':', ", ");

        // Шаблон компилируется из OUT_DIR, поэтому относительный путь к иконке
        // оттуда бы не разрешился — подставляем абсолютный. В .rc обратный слеш
        // внутри строки это escape-символ, поэтому удваиваем его.
        let icon_path = manifest_dir
            .join("assets")
            .join("icon.ico")
            .display()
            .to_string()
            .replace('\\', "\\\\");

        let template = std::fs::read_to_string(manifest_dir.join("windows/resource.rc.in"))
            .expect("read windows/resource.rc.in");
        let rendered = template
            .replace("@ICON_PATH@", &icon_path)
            .replace("@VERSION_COMMA@", &format!("{major},{minor},{patch},0"))
            .replace("@VERSION@", &version)
            .replace("@COMPANY@", &company);

        let rc_path = out_dir.join("resource.rc");
        std::fs::write(&rc_path, rendered).expect("write generated resource.rc");

        embed_resource::compile(&rc_path, embed_resource::NONE);
    }
}
