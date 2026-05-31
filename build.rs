fn main() {
    // Solo compilar y enlazar el recurso de icono si el sistema operativo objetivo es Windows
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/fastyIcon.ico");
        res.compile().unwrap();
    }
}
