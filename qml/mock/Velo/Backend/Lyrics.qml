pragma Singleton
import QtQuick

// MOCK of Velo.Backend.Lyrics (docs/CONTRATO.md). Lines are invented for the preview.
QtObject {
    property string status: "synced"       // "loading" | "synced" | "plain" | "none" | "instrumental"
    property int activeIndex: 8
    property string source: "spotify"      // "spotify" | "lrclib" | "" (cargando o sin letras)
    property string plainText: "La marea sube despacio\nY borra el nombre en la arena\n\nNadie mira hacia el faro\nCuando el agua está serena"

    property ListModel model: ListModel {
        ListElement { text: "La marea sube despacio"; timeMs: 14200 }
        ListElement { text: "Y borra el nombre en la arena"; timeMs: 19800 }
        ListElement { text: "Nadie mira hacia el faro"; timeMs: 25400 }
        ListElement { text: "Cuando el agua está serena"; timeMs: 31000 }
        ListElement { text: "Guardo el ruido de las olas"; timeMs: 38600 }
        ListElement { text: "En un frasco junto a la cama"; timeMs: 44100 }
        ListElement { text: "Para abrirlo en las noches largas"; timeMs: 51300 }
        ListElement { text: "Cuando el cuarto no dice nada"; timeMs: 57800 }
        ListElement { text: "Y si me buscas, sigo en la orilla"; timeMs: 108900 }
        ListElement { text: "Contando barcos que no regresan"; timeMs: 115200 }
        ListElement { text: "Con la sal pegada en la camisa"; timeMs: 121600 }
        ListElement { text: "Y una canción que aún no termina"; timeMs: 128000 }
        ListElement { text: "Azul cobalto"; timeMs: 136400 }
        ListElement { text: "Azul de madrugada"; timeMs: 140100 }
        ListElement { text: "Todo lo que se fue con la marea"; timeMs: 146900 }
        ListElement { text: "Vuelve cuando nadie lo esperaba"; timeMs: 153300 }
        ListElement { text: "Dejo la luz del porche encendida"; timeMs: 171000 }
        ListElement { text: "Por si el viento cambia de idea"; timeMs: 177400 }
        ListElement { text: "Y si me buscas, sigo en la orilla"; timeMs: 184200 }
        ListElement { text: "Hasta que el mar se quede quieto"; timeMs: 190800 }
    }
}
