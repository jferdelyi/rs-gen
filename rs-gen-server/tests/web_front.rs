//! Accessibility rules of the web front (`simple-web-front`), checked on its files.
//!
//! The front has no build step and no JavaScript test suite: these tests read its HTML, CSS and
//! JavaScript as text. They keep what an audit in a browser measured (WCAG 2.2 AA): contrast of
//! the colors, visible focus, modal dialogs, announced messages, labelled fields, rows of
//! buttons that fit a 320 px wide screen.

use std::collections::HashMap;
use std::path::PathBuf;

/// Reads a file of the web front.
fn front(file: &str) -> String {
	let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "simple-web-front", file].iter().collect();
	std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {}", path.display(), e))
}

/// The CSS without its comments.
fn without_comments(css: &str) -> String {
	let mut out = String::new();
	let mut rest = css;
	while let Some(start) = rest.find("/*") {
		out.push_str(&rest[..start]);
		rest = rest[start + 2..].find("*/").map_or("", |end| &rest[start + 2 + end + 2..]);
	}
	out.push_str(rest);
	out
}

/// A CSS rule: its selector and its declarations (property, value).
struct Rule {
	selector: String,
	declarations: Vec<(String, String)>,
}

/// The rules of a style sheet, those of `@media` blocks included.
fn rules(css: &str) -> Vec<Rule> {
	let css = without_comments(css);
	let mut found = Vec::new();
	collect_rules(&css, &mut found);
	found
}

fn collect_rules(css: &str, found: &mut Vec<Rule>) {
	let mut rest = css;
	while let Some(open) = rest.find('{') {
		let selector = rest[..open].trim().to_string();
		// The block ends at the brace that closes this one
		let mut depth = 0;
		let mut close = None;
		for (i, c) in rest[open..].char_indices() {
			match c {
				'{' => depth += 1,
				'}' => {
					depth -= 1;
					if depth == 0 {
						close = Some(open + i);
						break;
					}
				}
				_ => {}
			}
		}
		let close = close.unwrap_or_else(|| panic!("unclosed block after '{selector}'"));
		let body = &rest[open + 1..close];
		if body.contains('{') {
			collect_rules(body, found);
		} else {
			let declarations = body
				.split(';')
				.filter_map(|d| d.split_once(':'))
				.map(|(p, v)| (p.trim().to_string(), v.trim().to_string()))
				.filter(|(p, _)| !p.is_empty())
				.collect();
			found.push(Rule { selector, declarations });
		}
		rest = &rest[close + 1..];
	}
}

/// The custom properties (`--name: value`) of the `:root` rule of the theme.
fn theme() -> HashMap<String, String> {
	rules(&front("theme.css"))
		.into_iter()
		.filter(|r| r.selector == ":root")
		.flat_map(|r| r.declarations)
		.filter(|(p, _)| p.starts_with("--"))
		.collect()
}

/// An opaque color in sRGB.
#[derive(Clone, Copy, Debug)]
struct Color(f64, f64, f64);

/// Reads `#rgb` or `#rrggbb`.
fn hex(value: &str) -> Option<Color> {
	let digits = value.strip_prefix('#')?;
	let channel = |s: &str| u8::from_str_radix(s, 16).ok().map(f64::from);
	match digits.len() {
		3 => {
			let d: Vec<String> = digits.chars().map(|c| format!("{c}{c}")).collect();
			Some(Color(channel(&d[0])?, channel(&d[1])?, channel(&d[2])?))
		}
		6 => Some(Color(channel(&digits[0..2])?, channel(&digits[2..4])?, channel(&digits[4..6])?)),
		_ => None,
	}
}

/// Relative luminance, as WCAG 2.2 defines it.
fn luminance(c: Color) -> f64 {
	let linear = |v: f64| {
		let v = v / 255.0;
		if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
	};
	0.2126 * linear(c.0) + 0.7152 * linear(c.1) + 0.0722 * linear(c.2)
}

/// Contrast ratio of two colors, from 1 to 21.
fn contrast(a: Color, b: Color) -> f64 {
	let (la, lb) = (luminance(a), luminance(b));
	(la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// A start tag of an HTML page: its name and its attributes.
struct Tag {
	name: String,
	attributes: HashMap<String, String>,
}

impl Tag {
	fn get(&self, attribute: &str) -> Option<&str> {
		self.attributes.get(attribute).map(String::as_str)
	}

	fn has_class(&self, class: &str) -> bool {
		self.get("class").is_some_and(|c| c.split_whitespace().any(|x| x == class))
	}
}

/// The start and end tags of a page, in order; an end tag has a name starting with '/'.
fn tags(html: &str) -> Vec<Tag> {
	let mut found = Vec::new();
	let mut rest = html;
	while let Some(start) = rest.find('<') {
		rest = &rest[start + 1..];
		if rest.starts_with("!--") {
			rest = rest.find("-->").map_or("", |end| &rest[end + 3..]);
			continue;
		}
		let end = rest.find('>').expect("unclosed tag");
		let inside = &rest[..end];
		rest = &rest[end + 1..];
		if inside.starts_with('!') {
			continue;
		}
		let name_end = inside.find(|c: char| c.is_whitespace()).unwrap_or(inside.len());
		let name = inside[..name_end].trim_end_matches('/').to_lowercase();
		let mut attributes = HashMap::new();
		let mut attrs = inside[name_end..].trim();
		while !attrs.is_empty() {
			let key_end = attrs.find(|c: char| c == '=' || c.is_whitespace()).unwrap_or(attrs.len());
			let key = attrs[..key_end].to_lowercase();
			attrs = attrs[key_end..].trim_start();
			let mut value = String::new();
			if let Some(after) = attrs.strip_prefix('=') {
				let after = after.trim_start();
				let quote = after.chars().next().unwrap_or('"');
				let body = &after[1..];
				let close = body.find(quote).expect("unclosed attribute value");
				value = body[..close].to_string();
				attrs = body[close + 1..].trim_start();
			}
			if !key.is_empty() && key != "/" {
				attributes.insert(key, value);
			}
		}
		found.push(Tag { name, attributes });
	}
	found
}

const PAGES: [&str; 2] = ["index.html", "admin.html"];
const STYLES: [&str; 3] = ["theme.css", "style.css", "admin.css"];

/// Pairs of theme colors that must reach a contrast: text on its background (4.5:1, 1.4.3),
/// field borders and the focus ring on what surrounds them (3:1, 1.4.11).
const CONTRASTS: &[(&str, &str, f64)] = &[
	("--text", "--surface", 4.5),
	("--text", "--page", 4.5),
	("--muted", "--surface", 4.5),
	("--terminal-text", "--terminal", 4.5),
	("--terminal-muted", "--terminal", 4.5),
	("--terminal-user", "--terminal", 4.5),
	("--terminal-path", "--terminal", 4.5),
	("--on-primary", "--primary", 4.5),
	("--on-primary", "--primary-hover", 4.5),
	("--on-action", "--action", 4.5),
	("--on-action", "--action-hover", 4.5),
	("--on-danger", "--danger", 4.5),
	("--on-danger", "--danger-hover", 4.5),
	("--danger", "--surface", 4.5),
	("--success", "--surface", 4.5),
	("--busy", "--surface", 4.5),
	("--error-text", "--error-surface", 4.5),
	("--warning-text", "--warning-surface", 4.5),
	("--success-text", "--success-surface", 4.5),
	("--on-tooltip", "--tooltip", 4.5),
	("--field-border", "--surface", 3.0),
	("--field-border", "--page", 3.0),
	("--focus", "--surface", 3.0),
	("--focus", "--page", 3.0),
];

#[test]
fn every_color_of_the_front_is_a_token_of_the_theme() {
	for page in PAGES {
		let html = front(page);
		let theme = html.find("href=\"theme.css\"").unwrap_or_else(|| panic!("{page} does not load theme.css"));
		let style = html.find("href=\"style.css\"").unwrap_or_else(|| panic!("{page} does not load style.css"));
		assert!(theme < style, "{page} loads theme.css after style.css");
	}
	let mut literals = Vec::new();
	for file in ["style.css", "admin.css"] {
		for rule in rules(&front(file)) {
			for (property, value) in &rule.declarations {
				let lower = value.to_lowercase();
				let named = lower.split(|c: char| !c.is_ascii_alphabetic()).any(|w| matches!(w, "black" | "white" | "red" | "green" | "gray" | "grey"));
				if lower.contains('#') || lower.contains("rgb") || lower.contains("hsl") || named {
					literals.push(format!("{file}: {} {{ {property}: {value} }}", rule.selector));
				}
			}
		}
	}
	assert!(literals.is_empty(), "colors written outside theme.css:\n{}", literals.join("\n"));
}

#[test]
fn the_colors_of_the_theme_have_enough_contrast() {
	let theme = theme();
	let color = |token: &str| {
		let value = theme.get(token).unwrap_or_else(|| panic!("theme.css has no {token}"));
		hex(value).unwrap_or_else(|| panic!("{token} is not an opaque #hex color: {value}"))
	};
	let failures: Vec<String> = CONTRASTS
		.iter()
		.map(|&(fore, back, minimum)| (fore, back, minimum, contrast(color(fore), color(back))))
		.filter(|&(_, _, minimum, ratio)| ratio < minimum)
		.map(|(fore, back, minimum, ratio)| format!("{fore} on {back}: {ratio:.2}:1, at least {minimum}:1"))
		.collect();
	assert!(failures.is_empty(), "contrast too low:\n{}", failures.join("\n"));
}

#[test]
fn the_keyboard_focus_is_drawn_with_the_focus_color() {
	let drawn = STYLES.iter().flat_map(|f| rules(&front(f))).any(|rule| {
		rule.selector.contains(":focus-visible")
			&& rule.declarations.iter().any(|(p, v)| p == "outline" && v.contains("var(--focus)") && !v.contains("none"))
	});
	assert!(drawn, "no :focus-visible rule draws an outline with var(--focus)");
}

#[test]
fn the_settings_dialog_opens_on_its_title() {
	// Safari draws the keyboard focus on the first control a dialog focuses, even when the
	// dialog was opened with the mouse: the settings open on their title, which is not a control
	// and draws no ring, and Tab leads to the first slider
	let tags = tags(&front("index.html"));
	let dialog = tags.iter().position(|t| t.get("id") == Some("settings-modal")).expect("no #settings-modal in index.html");
	let first = tags[dialog + 1..].iter().find(|t| t.get("autofocus").is_some()).expect("nothing has autofocus in the settings");
	assert_eq!(first.get("id"), Some("settings-title"), "the settings open on another element than their title");
	assert_eq!(first.get("tabindex"), Some("-1"), "the title can be reached with Tab");
	let no_ring = STYLES.iter().flat_map(|f| rules(&front(f))).any(|rule| {
		rule.selector.split(',').any(|s| s.trim() == "[tabindex=\"-1\"]:focus")
			&& rule.declarations.iter().any(|(p, v)| p == "outline" && v == "none")
	});
	assert!(no_ring, "an element focused by script only draws the focus ring");
}

#[test]
fn every_dialog_is_a_native_modal_dialog() {
	for page in PAGES {
		let tags = tags(&front(page));
		let dialogs: Vec<&Tag> = tags.iter().filter(|t| t.has_class("modal")).collect();
		assert!(!dialogs.is_empty(), "{page} has no dialog");
		for dialog in dialogs {
			let id = dialog.get("id").unwrap_or("?");
			assert_eq!(dialog.name, "dialog", "{page}: #{id} is a <{}>, not a <dialog>", dialog.name);
			let title = dialog.get("aria-labelledby").unwrap_or_else(|| panic!("{page}: #{id} has no aria-labelledby"));
			assert!(tags.iter().any(|t| t.get("id") == Some(title)), "{page}: #{id} is labelled by a missing #{title}");
		}
	}
	// Opened as modal dialogs: the page behind stops answering and Escape closes them
	let script = front("script.js");
	let admin = front("admin.js");
	for (file, code, dialog) in [("script.js", &script, "settingsModal"), ("admin.js", &admin, "labelModal"), ("admin.js", &admin, "deleteModal")] {
		assert!(code.contains(&format!("{dialog}.showModal()")), "{file} does not open {dialog} with showModal()");
		assert!(!code.contains(&format!("{dialog}.classList")), "{file} still shows {dialog} through its classes");
	}
}

#[test]
fn every_message_of_the_pages_is_announced() {
	let admin = tags(&front("admin.html"));
	let messages: Vec<&Tag> = admin.iter().filter(|t| (t.has_class("message") || t.get("id") == Some("report")) && t.get("id").is_some()).collect();
	assert!(messages.len() >= 7, "only {} messages found in admin.html", messages.len());
	for message in messages {
		let id = message.get("id").unwrap_or("?");
		assert!(matches!(message.get("role"), Some("status" | "alert")), "admin.html: #{id} has no role status or alert");
		// A live region out of the accessibility tree is not announced when it comes back
		assert!(!message.has_class("hidden"), "admin.html: #{id} is hidden with display: none");
	}
	assert!(!front("admin.js").contains("report.classList.add(\"hidden\")"), "admin.js hides the report with display: none");
	// The generated words appear letter by letter: the whole word is announced apart
	let index = tags(&front("index.html"));
	assert!(index.iter().any(|t| t.get("role") == Some("status")), "index.html has no status region");
}

#[test]
fn every_field_has_a_visible_label() {
	for page in PAGES {
		let tags = tags(&front(page));
		let labelled: Vec<&str> = tags.iter().filter(|t| t.name == "label").filter_map(|t| t.get("for")).collect();
		let ids: Vec<&str> = tags.iter().filter_map(|t| t.get("id")).collect();
		// Inside a <label> without `for`, only the first field takes its text as label
		let mut wrapping: Vec<bool> = Vec::new();
		let mut missing = Vec::new();
		for tag in &tags {
			match tag.name.as_str() {
				"label" if tag.get("for").is_none() => wrapping.push(true),
				"label" => wrapping.push(false),
				"/label" => {
					wrapping.pop();
				}
				"input" | "select" | "textarea" => {
					let kind = tag.get("type").unwrap_or("text");
					if matches!(kind, "hidden" | "submit" | "button") || tag.has_class("visually-hidden") {
						continue;
					}
					let by_wrapper = wrapping.last_mut().is_some_and(|free| std::mem::replace(free, false));
					let by_for = tag.get("id").is_some_and(|id| labelled.contains(&id));
					let by_text = tag.get("aria-labelledby").is_some_and(|l| l.split_whitespace().all(|id| ids.contains(&id)));
					if !(by_wrapper || by_for || by_text) {
						missing.push(format!("<{} id=\"{}\">", tag.name, tag.get("id").unwrap_or("?")));
					}
				}
				_ => {}
			}
		}
		assert!(missing.is_empty(), "{page}: fields without a visible label: {}", missing.join(", "));
	}
	// The weights of the models are created by script.js, each with its label
	assert!(front("script.js").contains("label.htmlFor = slider.id"), "script.js does not tie each weight to its label");
}

#[test]
fn every_control_has_a_tooltip() {
	// A tooltip says what a control does: its own title, or the one of the <label> around it
	for page in PAGES {
		let tags = tags(&front(page));
		let mut labels: Vec<bool> = Vec::new();
		let mut missing = Vec::new();
		for tag in &tags {
			match tag.name.as_str() {
				"label" => labels.push(tag.get("title").is_some_and(|t| !t.trim().is_empty())),
				"/label" => {
					labels.pop();
				}
				"button" | "a" | "input" | "select" | "textarea" => {
					if tag.get("type") == Some("hidden") || tag.has_class("visually-hidden") {
						continue;
					}
					let own = tag.get("title").is_some_and(|t| !t.trim().is_empty());
					if !own && !labels.iter().any(|titled| *titled) {
						let name = tag.get("id").or(tag.get("href")).unwrap_or("?");
						missing.push(format!("<{} {}>", tag.name, name));
					}
				}
				_ => {}
			}
		}
		assert!(missing.is_empty(), "{page}: controls without a tooltip: {}", missing.join(", "));
	}
	// The controls created by the scripts: the weights of the models and the buttons of a model
	assert!(front("script.js").contains("slider.title = "), "script.js gives the weights no tooltip");
	assert!(front("admin.js").contains("button.title = title"), "admin.js gives the buttons of a model no tooltip");
}

#[test]
fn the_pages_draw_their_tooltips() {
	// The browser draws a title only after a long hover, never for the keyboard nor on a phone:
	// tooltip.js draws them instead, on hover and on focus (WCAG 1.4.13)
	for (page, own) in [("index.html", "script.js"), ("admin.html", "admin.js")] {
		let scripts: Vec<String> = tags(&front(page)).iter().filter(|t| t.name == "script").filter_map(|t| t.get("src").map(str::to_string)).collect();
		assert_eq!(scripts, ["tooltip.js", own], "{page} does not load tooltip.js before {own}");
	}
	let tooltip = front("tooltip.js");
	for (code, what) in [
		("removeAttribute(\"title\")", "the browser draws its own tooltip as well"),
		("aria-describedby", "the screen readers lose the text of the tooltip"),
		("\"Escape\"", "Escape does not hide the tooltip"),
		("\"focusin\"", "the keyboard focus shows no tooltip"),
		("MutationObserver", "the controls added by the scripts get no tooltip"),
	] {
		assert!(tooltip.contains(code), "tooltip.js: {what}");
	}
}

#[test]
fn every_button_has_the_font_size_of_the_links_drawn_as_buttons() {
	// WebKit gives a button the small font of the system (11 px), Chromium and Firefox 13.333 px:
	// in Safari, the Log out button was smaller than the Back link next to it
	let all: Vec<Rule> = STYLES.iter().flat_map(|f| rules(&front(f))).collect();
	let size_of = |selector: &str| {
		all.iter()
			.filter(|r| r.selector.split(',').any(|s| s.trim() == selector))
			.flat_map(|r| r.declarations.iter())
			.find(|(p, _)| p == "font-size")
			.map(|(_, v)| v.clone())
	};
	let link = size_of(".link-btn").expect("no font size for .link-btn");
	assert_eq!(size_of("button").as_deref(), Some(link.as_str()), "the buttons have no font size of their own");
}

#[test]
fn a_row_of_fixed_buttons_shrinks_on_a_narrow_screen() {
	// WebKit sizes a row of buttons from their width, not from their flex basis: without a
	// width, the Log out button left its card in Safari. On a narrow screen the width gives way:
	// at 320 px a fixed width made the admin page 356 px wide, Back and Log out out of the
	// screen (1.4.10)
	let widths = |css: &str| -> Vec<String> {
		rules(css)
			.into_iter()
			.filter(|r| r.selector.split(',').any(|s| s.trim() == ".fixed-btn"))
			.flat_map(|r| r.declarations)
			.filter(|(p, _)| p == "width" || p == "min-width")
			.map(|(p, v)| format!("{p}: {v}"))
			.collect()
	};
	let admin = without_comments(&front("admin.css"));
	let (wide, narrow) = admin.split_once("@media (max-width: 600px)").expect("no narrow screen block in admin.css");
	assert_eq!(widths(wide), ["width: 160px"], "on a wide screen");
	assert_eq!(widths(narrow), ["width: auto"], "on a narrow screen");
	for file in ["theme.css", "style.css"] {
		assert!(widths(&front(file)).is_empty(), "{file} gives .fixed-btn a width");
	}
}

#[test]
fn the_parsers_of_these_tests_read_the_files() {
	// Positive controls: without them, a check that finds nothing would prove nothing
	let style = rules(&front("style.css"));
	assert!(style.iter().any(|r| r.selector == ".card" && r.declarations.iter().any(|(p, _)| p == "border-radius")));
	assert!(style.iter().any(|r| r.selector == ".terminal"), "no .terminal rule read in style.css");
	let admin = tags(&front("admin.html"));
	assert!(admin.iter().any(|t| t.name == "button" && t.get("id") == Some("logout-btn")));
	assert!(admin.iter().any(|t| t.name == "/label"), "no end tag read in admin.html");
	assert!((contrast(Color(0.0, 0.0, 0.0), Color(255.0, 255.0, 255.0)) - 21.0).abs() < 1e-9);
	assert!((contrast(hex("#777").expect("#777"), hex("#ffffff").expect("#fff")) - 4.48).abs() < 0.01);
}
