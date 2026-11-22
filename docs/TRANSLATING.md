# Translating Lounge

Lounge is written in English and translated into German, Spanish, French, Italian, Dutch,
Brazilian Portuguese and European Portuguese. It follows the TV's language, and Settings has a
Language choice for a TV set to something else. The phone app follows the TV.

This page says where the text lives, how to write text so it can be translated, how each
language should sound, and how to add a language.

## Where the text lives

| What | Text is written in | Translations |
|---|---|---|
| The TV app (`android/`) | `res/values/*.xml` in each module | `res/values-de/`, `values-es/`, `values-fr/`, `values-it/`, `values-nl/`, `values-pt/` (Brazil), `values-pt-rPT/` |
| The core (`core/`) | the Rust source, inside `t!` | `core/locales/<tag>.po` |
| The plugin SDK (`plugins/sdk/`) | the Rust source, inside `t!` | `plugins/sdk/locales/<tag>.po` |
| Each plugin (`plugins/<name>/`) | the Rust source, inside `t!` | `plugins/<name>/locales/<tag>.po` |
| The phone app (`companion/`) | the TypeScript and Svelte source, inside `t(...)` | `companion/locales/<tag>.po` |

The tags are `de`, `es`, `fr`, `it`, `nl`, `pt-BR` and `pt-PT`. Text with no translation is
shown in English, so a half-finished language never shows a blank.

The TV decides the language. Its resources carry a `language_tag` string, the home screen hands
that tag to the core with `set_locale`, the core writes everything it says in that language and
gives the tag to each plugin (`setting("$locale")`) and to the phone (`device.lang`).

`tools/words.py check` finds text that is missing, left over, untranslated, or that fills
different places than its English. CI runs it. `tools/words.py sync` rewrites the `.po`
catalogs from the source: new text is added empty, text that is gone is dropped, and
translations are kept.

## Writing text that can be translated

These hold everywhere: Kotlin, Rust, the phone app and plugins.

- **Whole sentences.** Never build a sentence from pieces. Word order, gender and case differ
  between languages, so `"Added to " + name + " today"` cannot be translated; a sentence with a
  place for the name can.
- **Named places.** A value goes in a named place, `{source}` in a catalog and `%1$s` in
  Android resources, so a translation can move it.
- **No grammar in code.** No `a`/`an` choice, no adding `s`, no lowercasing a word to fit it
  inside a sentence, no `to_uppercase` on the first letter. Write each wording out in full.
- **Counts go through the plural forms.** `tn!(n, "{n} app", "{n} apps")`, a `<plurals>`
  resource, or `tn(n, ...)` on the phone. Never `if n == 1`.
- **A word that fills a place must stand alone.** "Add the {label} in the plugin's settings"
  only works in English; other languages need the label's gender and case. Prefer a sentence
  where the name sits after a colon or in quotes, or pass a whole translated phrase.
- **One meaning per text.** When the same English words mean two things ("Play" a game, "Play"
  a film), give them a context: `tc!("games", "Play")` or two resource names.
- **Names stay as they are.** Lounge, Jellyfin, Plex, HDMI, Shizuku and other product names are
  not translated, and neither is anything a server or an app sent (titles, channel names).
- **Leave out what people never read.** Log lines, developer errors, test strings and
  identifiers stay plain English literals.
- **No space at either end** of a text; put spacing in the code or the layout.
- **Lists and numbers.** `words::list(&names)` writes "A, B and C" as the language does, and
  `words::decimal(7.5, 1)` uses its decimal mark. Clock times follow the TV's 24-hour setting.

### In Rust: the core, the SDK and plugins

```rust
use lounge_words::{t, tc, tn};          // in a plugin: use lounge_plugin_sdk::{t, tc, tn};

t!("Continue watching")                                  // a String
t!("Added to {source} today", source = name)             // values fill named places
tc!("games", "Play")                                     // a context first
tn!(count, "{n} channel", "{n} channels")                // {n} is the count
tn!(left, "{n} minute left of {title}", "{n} minutes left of {title}", title = title)
```

The first text must be a literal: the tools read it from the source. Text kept in a constant
or a table is named with `mark!("Sunny")`, which leaves it as written, and said with
`words::tr(text)` where it is shown. A `const` that held a sentence becomes a function that
returns `t!(...)`.

A line that starts `// Translators:` directly above a call is copied into the catalogs as a
note for translators. Use it when the text alone does not say where it appears or what a place
holds.

Tests run in English, so their expected strings stay as they are.

A plugin ships its catalogs by adding `sdk::locales!();` inside its `impl Plugin`, which reads
`locales/<tag>.po` beside its `Cargo.toml` for the languages Lounge ships; name the tags
(`sdk::locales!("de", "sv")`) to ship others. Its manifest is written with `t!` like the rest,
and the SDK records what each language says for the name, description, row titles, labels and
help. PLUGINS.md has the details.

### On the phone

```ts
import { t, tc, tn } from './lang.svelte';

t('Hide')
t('Opened {when}', { when })
tn(count, '{n} app', '{n} apps')
```

Call these where the text is drawn, in the markup or in a function, never at the top of a
module: text read once at load would stay in the language the page started in. A table of
labels holds `mark('...')` text and goes through `tr(...)` when shown, or becomes a function.
No template strings with `${}` inside `t`.

### In the TV app

Text lives in string resources, in the module that shows it. Each source file has its own
resource file, named after it (`Sheets.kt` → `res/values/strings_sheets.xml`), and its names
start with the same word (`sheets_rows_title`). Words many screens share, such as Cancel, live
in `core/design` (`tv.lounge.design.R.string.common_cancel`).

```kotlin
stringResource(R.string.sheets_rows_title)
stringResource(R.string.plugins_set_up, plugin.name)             // %1$s
pluralStringResource(R.plurals.apps_count, count, count)          // %1$d
context.getString(R.string.update_failed)                          // outside a composable
```

- In a composable always use `stringResource` and `pluralStringResource`, so text changes at
  once when the language does. Do not keep text in `remember`.
- Code with no composable and no `Context` at hand returns a resource id (`@StringRes`), or a
  small value that the screen turns into text, or takes `Resources` as a parameter.
- Places are numbered: `%1$s`, `%2$d`. A literal percent sign is `%%`.
- Apostrophes and quotes are escaped: `Don\'t`, `\"`. An `@` or `?` at the start is `\@`, `\?`.
- A resource that is not text for people (`translatable="false"`) stays in `values/` only.
- A `<plurals>` has `one` and `other` in English, German and Dutch. Spanish, French, Italian
  and Portuguese also take `many`, which reads the same as `other`.

## How each language sounds

DESIGN.md §10 is the voice: sentence case, plain verbs, present tense, buttons that say what
will happen, errors that state the cause and the fix, no exclamation marks, no emoji. A
translation keeps that voice in its own idiom. It is written as a native speaker would write
it for a TV in their living room, never word for word.

| | Addressing the person | Notes |
|---|---|---|
| German | `du`, lowercase | Buttons are infinitives ("Öffnen", "Ausblenden"). Compound nouns over genitive chains. „Quotes“ like this. |
| Spanish | `tú` | Neutral Spanish that reads well in Spain and the Americas; avoid "vosotros" and regional words. Buttons are infinitives ("Abrir"). Opening ¿ and ¡ where the sentence needs them. «Quotes» or "quotes" as the English has them. |
| French | `vous` | Buttons are infinitives ("Ouvrir", "Masquer"). A no-break space (U+00A0) before `:` `;` `?` `!` and inside « quotes ». Apostrophes are typographic (’). |
| Italian | `tu` | Buttons are imperatives, as Italian TVs and phones write them ("Apri", "Nascondi", "Annulla"). "Quotes" like the English. |
| Dutch | `je` | Buttons are infinitives ("Openen", "Verbergen"). English loanwords where Dutch TVs use them ("app", "streamen"). |
| Portuguese (Brazil) | `você` | Buttons are infinitives ("Abrir", "Ocultar"). "Tela", "aplicativo" or "app", "arquivo". |
| Portuguese (Portugal) | impersonal or formal third person, no `tu` | Buttons are infinitives ("Abrir", "Ocultar"). "Ecrã", "aplicação", "ficheiro", "definições". |

Every language keeps:

- **Sentence case**, as far as the language has it. German nouns keep their capitals; nothing
  is set in capitals for emphasis.
- **The length.** A TV screen has little room. A button label stays within a few words; if the
  natural translation is much longer than the English, find a shorter one.
- **The same name for the same thing**, everywhere: on the TV, on the phone and in every
  plugin. The glossary below is the list.
- **The places.** Every `{name}` or `%1$s` in the English appears in the translation, spelled
  the same. A form for exactly one thing may spell the number out ("Ein Kanal").
- **Typography of the language**: its quotation marks, its decimal comma, its spacing before
  units. The ellipsis is one character (…), and a minus is U+2212 as in the English.
- **Words that stand alone need no agreement.** "On", "Off", "Hidden" and other one-word states
  are shown beside many different things, so they take a form that fits any gender and number,
  as in the glossary.
- **Names from Android's own screens** (Developer options, USB debugging, "Always allow") are
  written as the TV itself shows them in that language.
- **Names of themes, scenes and rows** are translated as names: short, evocative, capitalised
  as the language capitalises titles.
- **Some titles have a fixed width.** A note on the text says so, in pixels of Figtree
  SemiBold at 28 px: about 265 for a notice tile's title, which is roughly what "Turn on TV
  listings" takes in English, and about 140 for a small tile beside an icon. A longer title
  breaks at a word and ends in an ellipsis.
- **A plugin's row titles end the way the core expects.** The core has three patterns,
  "{row} on {plugin}", "{row} from {plugin}" and "{row} in {plugin}", and leaves that ending
  out where the plugin is already named. A row title such as "Recently added on Jellyfin" ends
  with one of them as the language's core catalog translates it.
- **Another product's words stay that product's.** A device that Home Assistant reports as on
  says so as Home Assistant's own interface does in that language, and its menus are named as
  it names them. The glossary's On and Off are for Lounge's own settings. Rows that an Android
  app publishes are that app's "channels" as Android TV calls them, which is not always the
  word for a TV channel.

### Glossary

The words Lounge uses for its own things. A translation uses the same word wherever the English
does.

| English | What it is | de | es | fr | it | nl | pt-BR | pt-PT |
|---|---|---|---|---|---|---|---|---|
| Lounge | the product; never translated | Lounge | Lounge | Lounge | Lounge | Lounge | Lounge | Lounge |
| Stage | the large area at the top of the home screen with one featured thing; "the Stage", "Pin to the Stage" | Bühne | Escenario | Scène | Palco | Podium | Palco | Palco |
| row | a line of tiles on the home screen | Reihe | fila | rangée | riga | rij | fileira | linha |
| tile | one item in a row | Kachel | tarjeta | vignette | riquadro | tegel | cartão | cartão |
| app |  | App | app | appli | app | app | app (o) | app (a) |
| Continue watching | the row of things to resume | Weiterschauen | Seguir viendo | Reprendre la lecture | Continua a guardare | Verder kijken | Continuar assistindo | Continuar a ver |
| Your apps | the row of pinned and recent apps | Deine Apps | Tus apps | Vos applis | Le tue app | Je apps | Seus apps | As suas apps |
| All apps | the screen listing every app | Alle Apps | Todas las apps | Toutes les applis | Tutte le app | Alle apps | Todos os apps | Todas as apps |
| Games | row | Spiele | Juegos | Jeux | Giochi | Games | Jogos | Jogos |
| Live now | the row of channels on air | Jetzt live | En directo ahora | En direct | In diretta | Nu live | Ao vivo agora | Em direto agora |
| Inputs | the TV's HDMI and other inputs | Eingänge | Entradas | Entrées | Ingressi | Ingangen | Entradas | Entradas |
| TV controls | the sheet with the TV's power and inputs | TV-Steuerung | Controles de la TV | Commandes TV | Controlli TV | Tv-bediening | Controles da TV | Controlos da TV |
| the TV | in sentences | der Fernseher | la TV | la TV | la TV | de tv | a TV | a TV |
| the phone | in sentences | das Handy | el teléfono | le téléphone | il telefono | de telefoon | o celular | o telemóvel |
| Up next | the next episode of something being watched | Als Nächstes | A continuación | À suivre | A seguire | Hierna | A seguir | A seguir |
| Pictures | artwork shown as wallpaper and by the screensaver | Bilder | Imágenes | Images | Immagini | Afbeeldingen | Imagens | Imagens |
| plugin | an add-on that brings rows, channels and guide data | Plugin (das, die Plugins) | plugin (el, los plugins) | plugin (le, les plugins) | plugin (il, i plugin) | plug-in (de, de plug-ins) | plugin (o, os plugins) | plugin (o, os plugins) |
| Phone companion | Lounge's page on a phone, and its row in Settings; used as a name | Lounge fürs Handy | Lounge en el teléfono | Lounge sur téléphone | Lounge sul telefono | Lounge op je telefoon | Lounge no celular | Lounge no telemóvel |
| Power Setup | the feature that uses the TV's USB debugging to do what an app cannot; used as a name | Erweiterte Einrichtung | Configuración avanzada | Configuration avancée | Configurazione avanzata | Geavanceerde configuratie | Configuração avançada | Configuração avançada |
| Settings | Lounge's settings | Einstellungen | Ajustes | Paramètres | Impostazioni | Instellingen | Configurações | Definições |
| Search | the button and the screen | Suche | Buscar | Rechercher | Cerca | Zoeken | Pesquisar | Pesquisar |
| TV guide / the guide | channels and their programmes | TV-Programm | Guía de TV | Guide TV | Guida TV | Tv-gids | Guia de TV | Guia de TV |
| programme | one broadcast in the guide | Sendung | programa | programme | programma | programma | programa | programa |
| channel |  | Sender | canal | chaîne | canale | zender | canal | canal |
| Notifications |  | Benachrichtigungen | Notificaciones | Notifications | Notifiche | Meldingen | Notificações | Notificações |
| Now playing | what an app is playing | Läuft gerade | En reproducción | Lecture en cours | In riproduzione | Speelt nu | Reproduzindo agora | A reproduzir |
| Theme | the colours of the whole interface | Design | Tema | Thème | Tema | Thema | Tema | Tema |
| Background | what is behind the home screen | Hintergrund | Fondo | Arrière-plan | Sfondo | Achtergrond | Plano de fundo | Fundo |
| scene | a slow moving picture drawn as a background or screensaver | Szene | escena | décor | scena | scène | cena | cena |
| Screensaver |  | Bildschirmschoner | Salvapantallas | Économiseur d’écran | Salvaschermo | Screensaver | Protetor de tela | Proteção de ecrã |
| Lite mode | fewer effects, for a slow TV | Leichter Modus | Modo ligero | Mode léger | Modalità leggera | Lichte modus | Modo leve | Modo leve |
| Reduce motion |  | Bewegung reduzieren | Reducir movimiento | Réduire les animations | Riduci movimento | Beweging verminderen | Reduzir movimento | Reduzir movimento |
| Sounds |  | Töne | Sonidos | Sons | Suoni | Geluiden | Sons | Sons |
| home screen | what the Home button opens | Startbildschirm | pantalla de inicio | écran d’accueil | schermata Home | startscherm | tela inicial | ecrã principal |
| Hide / Show / Hidden | taking something off the home screen and bringing it back | Ausblenden / Einblenden / Ausgeblendet | Ocultar / Mostrar / Oculto | Masquer / Afficher / Masqué | Nascondi / Mostra / Nascosto | Verbergen / Tonen / Verborgen | Ocultar / Mostrar / Oculto | Ocultar / Mostrar / Oculto |
| Pin / Unpin / pinned | keeping something in place | Anheften / Loslösen / angeheftet | Fijar / Desfijar / fijado | Épingler / Désépingler / épinglé | Fissa / Stacca / fissato | Vastmaken / Losmaken / vastgemaakt | Fixar / Desafixar / fixado | Fixar / Desafixar / fixado |
| On / Off | a setting's state, standing alone | An / Aus | Sí / No | Oui / Non | Sì / No | Aan / Uit | Sim / Não | Sim / Não |
| Refresh / Refresh now | a plugin fetching its data again | Aktualisieren / Jetzt aktualisieren | Actualizar / Actualizar ahora | Actualiser / Actualiser maintenant | Aggiorna / Aggiorna ora | Vernieuwen / Nu vernieuwen | Atualizar / Atualizar agora | Atualizar / Atualizar agora |
| Set up | a plugin, or Lounge itself | Einrichten | Configurar | Configurer | Configura | Instellen | Configurar | Configurar |
| Pair / pairing code | joining a phone to the TV | Koppeln / Kopplungscode | Vincular / código de vinculación | Associer / code d’association | Abbina / codice di abbinamento | Koppelen / koppelcode | Parear / código de pareamento | Emparelhar / código de emparelhamento |
| Remote | the phone's remote control | Fernbedienung | Control remoto | Télécommande | Telecomando | Afstandsbediening | Controle remoto | Comando |
| Layout | the phone's tab for arranging the home screen | Anordnung | Organización | Disposition | Disposizione | Indeling | Layout | Disposição |
| TV listings | Android's access to channels and programmes | TV-Programmdaten | programación de TV | programmes TV | programmi TV | tv-programmagegevens | programação de TV | programação de TV |
| Notification access | Android's name for it | Benachrichtigungszugriff | Acceso a notificaciones | Accès aux notifications | Accesso alle notifiche | Toegang tot meldingen | Acesso a notificações | Acesso a notificações |
| Usage access | Android's name for it | Zugriff auf Nutzungsdaten | Acceso a datos de uso | Accès aux données d’utilisation | Accesso ai dati di utilizzo | Gebruikstoegang | Acesso ao uso | Acesso de utilização |
| USB debugging | Android's name for it | USB-Debugging | Depuración por USB | Débogage USB | Debug USB | USB-foutopsporing | Depuração USB | Depuração USB |
| Developer options | Android's name for it | Entwickleroptionen | Opciones para desarrolladores | Options pour les développeurs | Opzioni sviluppatore | Ontwikkelaarsopties | Opções do desenvolvedor | Opções de programador |
| Updates | new versions of Lounge | Updates | Actualizaciones | Mises à jour | Aggiornamenti | Updates | Atualizações | Atualizações |
| About Lounge |  | Über Lounge | Acerca de Lounge | À propos de Lounge | Informazioni su Lounge | Over Lounge | Sobre o Lounge | Acerca do Lounge |
| Support Lounge | the page for giving | Lounge unterstützen | Apoyar a Lounge | Soutenir Lounge | Sostieni Lounge | Lounge steunen | Apoiar o Lounge | Apoiar o Lounge |
| Options | a tile's menu | Optionen | Opciones | Options | Opzioni | Opties | Opções | Opções |
| source | the app, server or service a title comes from | Quelle | fuente | source | sorgente | bron | fonte | fonte |
| Open / Watch / Continue | buttons | Öffnen / Ansehen / Fortsetzen | Abrir / Ver / Continuar | Ouvrir / Regarder / Reprendre | Apri / Guarda / Continua | Openen / Kijken / Verder kijken | Abrir / Assistir / Continuar | Abrir / Ver / Continuar |
| Play | a game / something to watch or hear | Spielen / Abspielen | Jugar / Reproducir | Jouer / Lire | Gioca / Riproduci | Spelen / Afspelen | Jogar / Reproduzir | Jogar / Reproduzir |
| OK, Back, Home | keys on the remote | OK, Zurück, Home | OK, Atrás, Inicio | OK, Retour, Accueil | OK, Indietro, Home | OK, Terug, Home | OK, Voltar, Início | OK, Voltar, Início |
| Hold OK | how a tile's menu is opened | OK gedrückt halten | Mantén pulsado OK | Maintenez OK | Tieni premuto OK | Houd OK ingedrukt | Mantenha OK pressionado | Mantenha OK premido |
| Season 2, Episode 4 / S2 E4 |  | Staffel 2, Folge 4 / S2 F4 | Temporada 2, episodio 4 / T2 E4 | Saison 2, épisode 4 / S2 É4 | Stagione 2, episodio 4 / S2 E4 | Seizoen 2, aflevering 4 / S2 A4 | Temporada 2, episódio 4 / T2 E4 | Temporada 2, episódio 4 / T2 E4 |
| film / series / episode |  | Film / Serie / Folge | película / serie / episodio | film / série / épisode | film / serie / episodio | film / serie / aflevering | filme / série / episódio | filme / série / episódio |
| server address | a plugin's setting | Serveradresse | dirección del servidor | adresse du serveur | indirizzo del server | serveradres | endereço do servidor | endereço do servidor |
| sign in |  | anmelden | iniciar sesión | se connecter | accedere | inloggen | entrar | iniciar sessão |

## Adding a language

1. Pick its tag: a language code, with a region only when Lounge ships two kinds of the
   language (`sv`, `pt-BR`).
2. `tools/words.py sync --add sv` starts a catalog for it beside every other one.
3. Add it to the lists that ship catalogs: `CATALOGS` in `core/lounge-core/src/lib.rs`,
   `LOCALES` and `locales!` in `plugins/sdk/src/lib.rs`. The phone app picks its catalog up
   from `companion/locales` by itself.
4. Add `values-sv/` beside each `values/` under `android/`, with every string and plural
   translated, and `language_tag` set to the tag in `core/design`.
5. Add it to `Language.shipped` in `android/core/design`, with its own name for itself, so
   Settings offers it.
6. If the language counts differently from English, check `core/lounge-words/src/plural.rs`
   and `forms` in `companion/src/lib/lang.svelte.ts` know its rule, and
   `PLURAL_HEADERS` in `tools/words.py`.
7. Translate, then run `tools/words.py check` until it is quiet.
8. Look at it on a TV. Text that is cut off or wraps badly is a bug in the translation.

The bundled typefaces cover Latin scripts, Vietnamese aside in the text face. A language in
another script falls back to the TV's own typeface until one is added for it, and a
right-to-left language also needs the layout mirrored and checked.

## Improving a translation

Edit the `.po` file or the `values-<tag>` resource and open a pull request. Say what read
wrong and why the new wording is better; a reviewer who does not speak the language can then
still follow it. `tools/words.py check` must stay quiet.

In a `.po` file an entry looks like this:

```po
#. A note from the code, when there is one
#: lounge-core/src/engine.rs
msgid "Added to {source} today"
msgstr "Heute zu {source} hinzugefügt"
```

Change only the `msgstr`. An entry marked `#, fuzzy` is one whose English changed since it was
translated: correct it and remove the mark.
