fn main() {
    // Estilo "fluent" (o do Windows) sem tema fixo: a janela alterna claro e escuro na hora.
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("falha ao compilar a interface");

    // Nome e ícone no Gerenciador de Tarefas e no Explorer: "Kage", com o símbolo.
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
