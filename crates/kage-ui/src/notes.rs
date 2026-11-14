//! Notas de cada versão, mostradas em "Novidades". Ficam dentro do .exe para abrir sem internet.
//! Ao lançar uma versão, acrescente a dela no topo e use o mesmo texto na release do GitHub.

pub struct Note {
    pub version: &'static str,
    pub date: &'static str,
    pub title: &'static str,
    pub items: &'static [&'static str],
}

pub const NOTES: &[Note] = &[Note {
    version: "0.1.0",
    date: "14 de novembro de 2026",
    title: "Primeira versão",
    items: &[
        "Wallpaper animado com vídeo, imagem e GIF: um em cada monitor, o mesmo em todos ou um só atravessando as telas.",
        "Um decodificador por arquivo, na placa de vídeo: um vídeo 4K nos dois monitores gasta perto de 200 MB de RAM e quase nada de CPU.",
        "Pausa sozinho com jogo em tela cheia, janela maximizada, bateria e área de trabalho remota, e devolve a memória de vídeo quando fica parado.",
        "Skins na área de trabalho, cada uma na sua janela: arrastar, travar, transparência, camadas, clicar através e escolha de monitor.",
        "A skin do player mostra a música que está tocando no Windows, com botões e barra de tempo que funcionam.",
        "Tema claro e escuro, iniciar com o Windows e atualização pelo próprio aplicativo.",
    ],
}];
