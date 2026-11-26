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
        version: "0.1.4",
        date: "25 de novembro de 2026",
        title: "Volta sozinho depois de trocar o driver de vídeo",
        items: &[
            "Atualizar ou reinstalar o driver da placa de vídeo não deixa mais a área de trabalho sem wallpaper: o Kage percebe que a placa reiniciou e volta com o wallpaper e as skins em poucos segundos, sem fechar e abrir nada.",
            "O wallpaper de cada monitor fica guardado pelo modelo da tela, e não mais pelo número que o Windows dá ao monitor, que muda quando o driver é trocado. Quem já usa o Kage não precisa escolher de novo: a configuração é convertida sozinha.",
            "Um erro que se repete nas skins entra uma vez só no registro do motor, em vez de encher o arquivo sem parar.",
        ],
    },
    Note {
        version: "0.1.3",
        date: "22 de novembro de 2026",
        title: "Skin nova no mesmo lugar",
        items: &[
            "A skin ligada pela aba Skins aparece na mesma altura do Relógio que já vem ligado: centralizada, a 35% da altura da tela.",
        ],
    },
    Note {
        version: "0.1.2",
        date: "22 de novembro de 2026",
        title: "Relógio do Kage",
        items: &[
            "O Kage agora traz uma skin própria, o Relógio: o dia da semana em letras largas, a data e o horário com um enfeite em volta.",
            "O dia da semana usa a Kage Display, uma fonte desenhada para o Kage. O horário e a data usam a Quicksand.",
            "Numa instalação nova, o Relógio já aparece no centro da tela principal. Quem já usa o Kage encontra o Relógio na aba Skins, pronto para ligar.",
            "Para mudar o tamanho, use Editar skin no clique direito do Relógio e troque a Escala e o EspacoDia (1.30 e 12.26 numa tela de 2560x1440, 1.06 e 10 numa de 1920x1080).",
        ],
    },
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
