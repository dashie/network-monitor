//! Templates and static assets of the two pages.
//!
//! A debug build reads them from the source tree on every request: save the HTML, the
//! stylesheet, or the script, refresh the browser, and the page shows the change. A release
//! build embeds them, so the executable still ships its pages and reads no file from disk.

/// A file the pages need.
#[derive(Clone, Copy)]
enum Asset {
    /// The timeline page.
    Page,
    /// The page of one target.
    Detail,
    /// The identity block both pages open with.
    Brand,
    /// The stylesheet both pages link.
    Stylesheet,
    /// The script both pages load.
    Script,
}

/// What the file is called, as a release binary ships it.
const PAGE: &str = include_str!("./templates/page.html");
const DETAIL: &str = include_str!("./templates/detail.html");
const BRAND: &str = include_str!("./templates/brand.html");
const STYLESHEET: &str = include_str!("./static/style.css");
const SCRIPT: &str = include_str!("./static/app.js");

impl Asset {
    /// The template of the page called `name`.
    fn from_name(name: &str) -> Self {
        match name {
            "page" => Self::Page,
            "detail" => Self::Detail,
            _ => panic!("unknown template: {name}"),
        }
    }

    /// The folder and the file that hold it in the source tree.
    #[cfg(debug_assertions)]
    fn path(self) -> (&'static str, &'static str) {
        match self {
            Self::Page => ("templates", "page.html"),
            Self::Detail => ("templates", "detail.html"),
            Self::Brand => ("templates", "brand.html"),
            Self::Stylesheet => ("static", "style.css"),
            Self::Script => ("static", "app.js"),
        }
    }

    /// The copy a release binary carries.
    fn embedded(self) -> &'static str {
        match self {
            Self::Page => PAGE,
            Self::Detail => DETAIL,
            Self::Brand => BRAND,
            Self::Stylesheet => STYLESHEET,
            Self::Script => SCRIPT,
        }
    }

    /// The asset as text.
    ///
    /// While developing, the file on disk wins, so a change needs neither a rebuild nor a
    /// restart. A file that cannot be read falls back to the embedded copy, so a page never
    /// fails for it.
    fn text(self) -> String {
        #[cfg(debug_assertions)]
        if let Some(text) = self.read() {
            return text;
        }
        self.embedded().to_string()
    }

    /// The file of the source tree, when it can be read.
    #[cfg(debug_assertions)]
    fn read(self) -> Option<String> {
        let (folder, file) = self.path();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/web")
            .join(folder)
            .join(file);
        std::fs::read_to_string(path).ok()
    }
}

/// The template of `name`, ready to be filled with its placeholders.
pub fn load_template(name: &str) -> String {
    Asset::from_name(name).text()
}

/// The identity block both pages open with, with no page-specific content.
pub fn brand() -> String {
    Asset::Brand.text()
}

/// The stylesheet both pages link, served at `/style.css`.
pub fn stylesheet() -> String {
    Asset::Stylesheet.text()
}

/// The script both pages load, served at `/app.js`.
pub fn script() -> String {
    Asset::Script.text()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// While developing, the files are read on every request: saving them is enough to see
    /// the change on the next refresh, with no rebuild and no restart.
    #[test]
    #[cfg(debug_assertions)]
    fn a_debug_build_reads_every_asset_from_the_source_tree() {
        for asset in [
            Asset::Page,
            Asset::Detail,
            Asset::Brand,
            Asset::Stylesheet,
            Asset::Script,
        ] {
            let (folder, file) = asset.path();
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/web")
                .join(folder)
                .join(file);
            let on_disk =
                std::fs::read_to_string(&path).expect("the asset sits in the source tree");
            assert_eq!(
                asset.text(),
                on_disk,
                "{folder}/{file} is read from the source tree"
            );
        }
    }

    /// The embedded copies are the pages and the assets both builds fall back to.
    #[test]
    fn the_embedded_copies_are_the_files() {
        assert!(PAGE.contains("<header class=\"masthead\">"));
        assert!(DETAIL.contains("<header class=\"masthead\">"));
        assert!(BRAND.contains("<h1>Network Monitor</h1>"));
        assert!(STYLESHEET.contains(".masthead"));
        assert!(SCRIPT.contains("Intl.DateTimeFormat"));
    }

    #[test]
    #[should_panic(expected = "unknown template: nope")]
    fn an_unknown_template_is_refused() {
        let _ = load_template("nope");
    }
}
