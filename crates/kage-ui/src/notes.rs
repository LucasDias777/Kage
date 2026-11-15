//! Notas de cada versão, mostradas em "Novidades". Ficam dentro do .exe para abrir sem internet.
//! Ao lançar uma versão, acrescente a dela no topo e use o mesmo texto na release do GitHub.

pub struct Note {
    pub version: &'static str,
    pub date: &'static str,
    pub title: &'static str,
    pub items: &'static [&'static str],
}

pub const NOTES: &[Note] = &[
    Note {
        version: "0.1.1",
        date: "14 de novembro de 2026",
        title: "Ícone na bandeja",
        items: &[
            "Ícone do Kage na bandeja do sistema: dois cliques abrem a janela, e o clique direito traz Abrir o Kage, Fechar todos os wallpapers, Pausar wallpapers, Alterar wallpaper, Customizar wallpaper, a atualização, Relatar um problema e Sair.",
            "Pausado pela bandeja, o ícone fica cinza e o wallpaper só volta quando você tira a marca; as skins continuam.",
            "Alterar wallpaper sorteia outro da sua biblioteca, em cada tela ou em todas, conforme a disposição.",
            "O motor procura versão nova sozinho, mesmo com a janela fechada, e avisa com um balão na bandeja.",
            "A opção Ícone da bandeja do sistema, em Configurações › Geral, agora funciona.",
            "Abrir o Kage de novo, pelo Menu Iniciar ou pela bandeja, traz a janela que já está aberta em vez de abrir outra.",
        ],
    },
    Note {
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
    },
];
