fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "RapidAgentTeamConfig");
        res.set("FileDescription", "Rapid Agent Team Configuration Tool");
        res.set("LegalCopyright", "Copyright (c) 2026 VastNext");
        let _ = res.compile();
    }
}
