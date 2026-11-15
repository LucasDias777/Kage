fn main() {
    // Nome e ícone no Gerenciador de Tarefas: o processo aparece como "Kage", com o símbolo.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/brand/kage.ico");
        res.set("FileDescription", "Kage");
        res.set("ProductName", "Kage");
        // Fornecedor na aba Inicializar do Gerenciador de Tarefas e nas propriedades do arquivo.
        res.set("CompanyName", "Lucas Dias");
        res.compile().expect("falha ao gravar o ícone e o nome no executável");
    }
}
