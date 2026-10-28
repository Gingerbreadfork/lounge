//! Sample library: a shelf of public-domain films with posters and stills from Wikimedia, so a fresh
//! install has something on the Stage and in the rows. A few are left part-watched. Opening
//! one shows its Wikipedia page, and a screensaver of a projector's beam comes with them.
//! Nothing here is anyone's viewing history.
use lounge_plugin_sdk::{self as sdk, host, Action, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, Shader};

pub struct Sample;

struct Film {
    id: &'static str,
    title: &'static str,
    year: u16,
    by: &'static str,
    minutes: i64,
    poster: &'static str,
    page: &'static str,
    /// A still for the landscape tiles of the films left part-way.
    still: Option<&'static str>,
    /// Minutes watched and hours since, for the films left part-way.
    watched: Option<(i64, i64)>,
}

const WIKI: &str = "https://en.wikipedia.org/wiki/";

const FILMS: &[Film] = &[
    Film { id: "metropolis", title: "Metropolis", year: 1927, by: "Fritz Lang", minutes: 153, poster: "https://upload.wikimedia.org/wikipedia/en/9/97/Metropolis_%28German_three-sheet_poster%29.jpg", page: "Metropolis_(1927_film)", still: Some("https://upload.wikimedia.org/wikipedia/commons/f/f9/Metropolis_%281927%29_-_Hel.jpg"), watched: Some((95, 44)) },
    Film { id: "his-girl-friday", title: "His Girl Friday", year: 1940, by: "Howard Hawks", minutes: 92, poster: "https://thumb.wikimedia.org/wikipedia/commons/thumb/2/21/His_Girl_Friday_%281940_poster%29_crop.jpg/960px-His_Girl_Friday_%281940_poster%29_crop.jpg", page: "His_Girl_Friday", still: Some("https://thumb.wikimedia.org/wikipedia/commons/thumb/9/98/His-Girl-Friday-Grant-Russell.jpg/1280px-His-Girl-Friday-Grant-Russell.jpg"), watched: Some((39, 20)) },
    Film { id: "charade", title: "Charade", year: 1963, by: "Stanley Donen", minutes: 113, poster: "https://thumb.wikimedia.org/wikipedia/commons/thumb/f/fd/Charade_%281963_poster%29.jpg/960px-Charade_%281963_poster%29.jpg", page: "Charade_(1963_film)", still: Some("https://thumb.wikimedia.org/wikipedia/commons/thumb/0/0b/Charade_1963_Audrey_Hepburn_and_Cary_Grant.jpg/1280px-Charade_1963_Audrey_Hepburn_and_Cary_Grant.jpg"), watched: Some((73, 2)) },
    Film { id: "the-general", title: "The General", year: 1926, by: "Buster Keaton", minutes: 75, poster: "https://upload.wikimedia.org/wikipedia/commons/0/0c/The_General_%281926%29_-_Movie_Poster_2.png", page: "The_General_(1926_film)", still: Some("https://thumb.wikimedia.org/wikipedia/commons/thumb/f/fb/Buster_Keaton_on_the_cow-catcher_in_The_General.jpg/1280px-Buster_Keaton_on_the_cow-catcher_in_The_General.jpg"), watched: Some((22, 70)) },
    Film { id: "nosferatu", title: "Nosferatu", year: 1922, by: "F. W. Murnau", minutes: 94, poster: "https://upload.wikimedia.org/wikipedia/commons/9/95/Max_Schreck_as_Count_Orlok_in_Nosferatu_%E2%80%93_Eine_Symphonie_des_Grauens_%281922%29.jpg", page: "Nosferatu", still: None, watched: None },
    Film { id: "sunrise", title: "Sunrise", year: 1927, by: "F. W. Murnau", minutes: 94, poster: "https://thumb.wikimedia.org/wikipedia/commons/thumb/2/27/Sunrise_-_A_Song_of_Two_Humans.jpg/960px-Sunrise_-_A_Song_of_Two_Humans.jpg", page: "Sunrise:_A_Song_of_Two_Humans", still: None, watched: None },
    Film { id: "sherlock-jr", title: "Sherlock Jr.", year: 1924, by: "Buster Keaton", minutes: 45, poster: "https://thumb.wikimedia.org/wikipedia/commons/thumb/c/c1/Sherlock_jr_poster.jpg/960px-Sherlock_jr_poster.jpg", page: "Sherlock_Jr.", still: Some("https://thumb.wikimedia.org/wikipedia/commons/thumb/7/77/Silent_film_theater_in_Sherlock_Jr.%2C_1924_%28cropped%29.jpg/1280px-Silent_film_theater_in_Sherlock_Jr.%2C_1924_%28cropped%29.jpg"), watched: Some((12, 96)) },
    Film { id: "the-lost-world", title: "The Lost World", year: 1925, by: "Harry O. Hoyt", minutes: 106, poster: "https://thumb.wikimedia.org/wikipedia/commons/thumb/7/76/The_Lost_World_%281925%29_-_film_poster.jpg/960px-The_Lost_World_%281925%29_-_film_poster.jpg", page: "The_Lost_World_(1925_film)", still: Some("https://upload.wikimedia.org/wikipedia/commons/3/3a/The_Lost_World_%281925%29_-_3.jpg"), watched: Some((40, 200)) },
    Film { id: "caligari", title: "The Cabinet of Dr. Caligari", year: 1920, by: "Robert Wiene", minutes: 76, poster: "https://thumb.wikimedia.org/wikipedia/commons/thumb/5/52/Das_Cabinet_des_Dr._Caligari.JPG/960px-Das_Cabinet_des_Dr._Caligari.JPG", page: "The_Cabinet_of_Dr._Caligari", still: None, watched: None },
    Film { id: "phantom", title: "The Phantom of the Opera", year: 1925, by: "Rupert Julian", minutes: 93, poster: "https://upload.wikimedia.org/wikipedia/commons/5/53/Phantom_of_the_opera_1925_poster.jpg", page: "The_Phantom_of_the_Opera_(1925_film)", still: Some("https://thumb.wikimedia.org/wikipedia/commons/thumb/3/3c/The_Phantom_of_the_Opera_%281925%29_-_9.jpg/1280px-The_Phantom_of_the_Opera_%281925%29_-_9.jpg"), watched: None },
    Film { id: "gold-rush", title: "The Gold Rush", year: 1925, by: "Charlie Chaplin", minutes: 95, poster: "https://thumb.wikimedia.org/wikipedia/commons/thumb/4/4d/Gold_rush_poster.jpg/960px-Gold_rush_poster.jpg", page: "The_Gold_Rush", still: Some("https://thumb.wikimedia.org/wikipedia/commons/thumb/4/48/Charlie_Chaplin%2C_Mack_Swain_and_Tiny_Sandford_in_The_Gold_Rush.jpg/1280px-Charlie_Chaplin%2C_Mack_Swain_and_Tiny_Sandford_in_The_Gold_Rush.jpg"), watched: Some((61, 150)) },
    Film { id: "the-kid", title: "The Kid", year: 1921, by: "Charlie Chaplin", minutes: 68, poster: "https://thumb.wikimedia.org/wikipedia/commons/thumb/b/b5/The_Kid_%281921%29_poster.jpg/960px-The_Kid_%281921%29_poster.jpg", page: "The_Kid_(1921_film)", still: Some("https://thumb.wikimedia.org/wikipedia/commons/thumb/7/79/Still_from_Charles_Chaplin_-_The_Kid_-_1921_-_Charles_Chaplin_Productions_-_EYE_FOT2424.jpg/1280px-Still_from_Charles_Chaplin_-_The_Kid_-_1921_-_Charles_Chaplin_Productions_-_EYE_FOT2424.jpg"), watched: Some((30, 120)) },
    Film { id: "potemkin", title: "Battleship Potemkin", year: 1925, by: "Sergei Eisenstein", minutes: 75, poster: "https://thumb.wikimedia.org/wikipedia/commons/thumb/8/85/Vintage_Potemkin.jpg/960px-Vintage_Potemkin.jpg", page: "Battleship_Potemkin", still: None, watched: None },
    Film { id: "joan-of-arc", title: "The Passion of Joan of Arc", year: 1928, by: "Carl Theodor Dreyer", minutes: 110, poster: "https://thumb.wikimedia.org/wikipedia/commons/thumb/d/d8/The_Passion_of_Joan_of_Arc_%281928%29_English_Poster.png/960px-The_Passion_of_Joan_of_Arc_%281928%29_English_Poster.png", page: "The_Passion_of_Joan_of_Arc", still: None, watched: None },
    Film { id: "plan-9", title: "Plan 9 from Outer Space", year: 1957, by: "Ed Wood", minutes: 79, poster: "https://thumb.wikimedia.org/wikipedia/commons/thumb/b/bf/Plan_9_Alternative_poster.jpg/960px-Plan_9_Alternative_poster.jpg", page: "Plan_9_from_Outer_Space", still: None, watched: None },
    Film { id: "scarlet-street", title: "Scarlet Street", year: 1945, by: "Fritz Lang", minutes: 102, poster: "https://thumb.wikimedia.org/wikipedia/commons/thumb/5/5d/Edward_G._Robinson_and_Joan_Bennett_in_%27Scarlet_Street%27%2C_1946.jpg/960px-Edward_G._Robinson_and_Joan_Bennett_in_%27Scarlet_Street%27%2C_1946.jpg", page: "Scarlet_Street", still: None, watched: None },
    Film { id: "wings", title: "Wings", year: 1927, by: "William A. Wellman", minutes: 144, poster: "https://thumb.wikimedia.org/wikipedia/commons/thumb/b/b3/Wings_%281927%29_poster.jpg/960px-Wings_%281927%29_poster.jpg", page: "Wings_(1927_film)", still: None, watched: None },
    Film { id: "the-circus", title: "The Circus", year: 1928, by: "Charlie Chaplin", minutes: 72, poster: "https://thumb.wikimedia.org/wikipedia/commons/thumb/4/4f/The_Circus_%281928%29_-_Hap_Hadley_poster.jpg/960px-The_Circus_%281928%29_-_Hap_Hadley_poster.jpg", page: "The_Circus_(1928_film)", still: None, watched: None },
];

impl sdk::Plugin for Sample {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.sample".into(),
            name: "Sample library".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Eighteen public-domain films with their posters, to see Lounge with something on it.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Screensaver],
            fetch_hosts: vec!["thumb.wikimedia.org".into(), "upload.wikimedia.org".into()],
            refresh_minutes: 24 * 60,
            rows: vec![RowSpec { id: "shelf".into(), title: "Film club".into(), style: RowStyle::Poster }],
            settings: vec![],
            homepage: None,
        }
    }

    fn refresh(_ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let now = host::now_ms();
        let mut items: Vec<Item> = FILMS
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let mut item = Item {
                    id: format!("film:{}", f.id),
                    title: f.title.into(),
                    subtitle: Some(format!("{}, {}", f.year, f.by)),
                    duration_ms: Some(f.minutes * 60_000),
                    poster: Some(f.poster.into()),
                    thumbnail: f.still.map(|still| still.into()),
                    action: Action::view(format!("{WIKI}{}", f.page)),
                    weight: (FILMS.len() - i) as i32,
                    ..Default::default()
                };
                match f.watched {
                    Some((minutes, hours_ago)) => {
                        item.kind = ItemKind::Resume;
                        item.position_ms = Some(minutes * 60_000);
                        item.last_used_ms = Some(now - hours_ago * 3_600_000);
                    }
                    None => item.row = Some("shelf".into()),
                }
                item
            })
            .collect();
        items.push(projector());
        Ok(items)
    }
}

/// The screensaver: the program is drawn by the TV, the plugin only chooses its colour and pace.
fn projector() -> Item {
    Item {
        id: "projector".into(),
        kind: ItemKind::Screensaver,
        title: "Projector".into(),
        subtitle: Some("Smoke in the beam, from the back row".into()),
        shader: Some(Shader::new(include_str!("projector.agsl")).float("drift", 0.08).color("beam", "#FFE2B8")),
        ..Default::default()
    }
}

sdk::export_plugin!(Sample);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_film_is_complete() {
        for f in FILMS {
            assert!(!f.id.is_empty() && !f.title.is_empty() && f.minutes > 0 && !f.poster.is_empty() && !f.page.is_empty(), "{}", f.title);
            if let Some((at, _)) = f.watched {
                assert!(at > 0 && at < f.minutes, "{}", f.title);
            }
        }
    }

    #[test]
    fn the_library_publishes_cleanly() {
        let items = <Sample as sdk::Plugin>::refresh(&RefreshContext::default()).unwrap();
        let rows: Vec<String> = <Sample as sdk::Plugin>::manifest().rows.into_iter().map(|r| r.id).collect();
        let ids: std::collections::HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids.len(), items.len());
        for it in &items {
            assert!(it.row.as_ref().is_none_or(|r| rows.contains(r)), "{}", it.title);
            if it.kind == ItemKind::Resume {
                assert!(it.thumbnail.is_some(), "{}", it.title);
            }
        }
    }
}
