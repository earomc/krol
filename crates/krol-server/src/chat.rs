//! Chat components (vanilla: `IChatComponent`, `ChatComponentText`,
//! `ChatComponentTranslation`, `ChatStyle` and their Gson serializer).

use krol_java::gson::JsonValue;

/// A component's content.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    /// Literal text.
    Text(String),
    /// A translation key with arguments.
    Translate {
        /// Translation key, e.g. `chat.type.text`.
        key: String,
        /// Arguments.
        with: Vec<Arg>,
    },
    /// A scoreboard value (`ChatComponentScore`).
    Score {
        /// Score holder name.
        name: String,
        /// Objective name.
        objective: String,
        /// The resolved value.
        value: String,
    },
    /// An entity selector (`ChatComponentSelector`).
    Selector(String),
}

/// `EnumChatFormatting` names, as accepted for `color`.
const FORMATTING_NAMES: [&str; 22] = [
    "black",
    "dark_blue",
    "dark_green",
    "dark_aqua",
    "dark_red",
    "dark_purple",
    "gold",
    "gray",
    "dark_gray",
    "blue",
    "green",
    "aqua",
    "red",
    "light_purple",
    "yellow",
    "white",
    "obfuscated",
    "bold",
    "strikethrough",
    "underline",
    "italic",
    "reset",
];

/// `JsonPrimitive.getAsString`.
fn json_as_string(v: &JsonValue) -> Result<String, String> {
    match v {
        JsonValue::String(s) => Ok(s.clone()),
        JsonValue::Int(i) => Ok(i.to_string()),
        JsonValue::Bool(b) => Ok(b.to_string()),
        _ => Err(format!("Not a primitive: {}", v.to_gson_string())),
    }
}

/// `JsonPrimitive.getAsBoolean`: booleans, or strings equal to "true" in any case.
fn json_as_bool(v: &JsonValue) -> Result<bool, String> {
    match v {
        JsonValue::Bool(b) => Ok(*b),
        JsonValue::String(s) => Ok(s.eq_ignore_ascii_case("true")),
        JsonValue::Int(_) => Ok(false),
        _ => Err(format!("Not a primitive: {}", v.to_gson_string())),
    }
}

/// Removes `§` formatting codes (`EnumChatFormatting.getTextWithoutFormattingCodes`).
pub fn strip_formatting(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{a7}'
            && let Some(&next) = chars.peek()
            && matches!(next.to_ascii_lowercase(), '0'..='9' | 'a'..='f' | 'k'..='o' | 'r')
        {
            chars.next();
            continue;
        }
        out.push(c);
    }
    out
}

/// A translation argument: vanilla passes either components or plain strings.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    /// A nested component.
    Component(Component),
    /// A raw string (serialized as a JSON string).
    Str(String),
}

/// A click or hover event.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Click event `(action, value)`.
    Click(String, String),
    /// Hover event `(action, value component)`.
    Hover(String, Box<Component>),
}

/// Formatting (vanilla: `ChatStyle`). `None` means "inherit".
#[derive(Debug, Clone, Default, PartialEq)]
#[allow(missing_docs)]
pub struct Style {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underlined: Option<bool>,
    pub strikethrough: Option<bool>,
    pub obfuscated: Option<bool>,
    /// Lowercase color name (e.g. `yellow`).
    pub color: Option<&'static str>,
    pub insertion: Option<String>,
    pub click: Option<(String, String)>,
    pub hover: Option<(String, Box<Component>)>,
}

impl Style {
    fn is_empty(&self) -> bool {
        *self == Style::default()
    }

    /// `ChatStyle.Serializer.deserialize` on a component object.
    fn from_json(json: &JsonValue) -> Result<Self, String> {
        let mut s = Style::default();
        let flag = |k: &str| json.get(k).map(json_as_bool).transpose();
        s.bold = flag("bold")?;
        s.italic = flag("italic")?;
        s.underlined = flag("underlined")?;
        s.strikethrough = flag("strikethrough")?;
        s.obfuscated = flag("obfuscated")?;
        if let Some(c) = json.get("color") {
            let name = json_as_string(c)?.to_lowercase();
            s.color = FORMATTING_NAMES.iter().find(|n| **n == name).copied();
        }
        if let Some(i) = json.get("insertion") {
            s.insertion = Some(json_as_string(i)?);
        }
        if let Some(click) = json
            .get("clickEvent")
            .filter(|c| matches!(c, JsonValue::Object(_)))
        {
            let action = click.get("action").map(json_as_string).transpose()?;
            let value = click.get("value").map(json_as_string).transpose()?;
            if let (Some(action), Some(value)) = (action, value)
                && matches!(
                    action.as_str(),
                    "open_url" | "run_command" | "suggest_command" | "change_page"
                )
            {
                s.click = Some((action, value));
            }
        }
        if let Some(hover) = json
            .get("hoverEvent")
            .filter(|c| matches!(c, JsonValue::Object(_)))
        {
            let action = hover.get("action").map(json_as_string).transpose()?;
            let value = match hover.get("value") {
                Some(v) => Some(Component::from_json(v)?),
                None => None,
            };
            if let (Some(action), Some(value)) = (action, value)
                && matches!(
                    action.as_str(),
                    "show_text" | "show_achievement" | "show_item" | "show_entity"
                )
            {
                s.hover = Some((action, Box::new(value)));
            }
        }
        Ok(s)
    }
}

/// A chat component.
#[derive(Debug, Clone, PartialEq)]
pub struct Component {
    /// Content.
    pub content: Content,
    /// Style.
    pub style: Style,
    /// Siblings (`extra`).
    pub extra: Vec<Component>,
}

impl Component {
    /// A plain text component.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: Content::Text(text.into()),
            style: Style::default(),
            extra: Vec::new(),
        }
    }

    /// A translation component.
    pub fn translate(key: impl Into<String>, with: Vec<Arg>) -> Self {
        Self {
            content: Content::Translate {
                key: key.into(),
                with,
            },
            style: Style::default(),
            extra: Vec::new(),
        }
    }

    /// Vanilla `getChatComponentFromNthArg`: an empty text component whose siblings are
    /// the words, with `" "` siblings between them.
    pub fn from_words(words: &[&str]) -> Self {
        let mut c = Self::text("");
        for (i, w) in words.iter().enumerate() {
            if i > 0 {
                c.extra.push(Self::text(" "));
            }
            c.extra.push(Self::text(*w));
        }
        c
    }

    /// Sets italic (builder style).
    #[must_use]
    pub fn italic(mut self) -> Self {
        self.style.italic = Some(true);
        self
    }

    /// Sets the color (builder style).
    #[must_use]
    pub fn color(mut self, color: &'static str) -> Self {
        self.style.color = Some(color);
        self
    }

    /// A player's display name as vanilla builds it (`EntityPlayer.getDisplayName`):
    /// insertion, `/msg` suggestion and a `show_entity` hover.
    pub fn player_name(name: &str, uuid: &str) -> Self {
        let mut c = Self::text(name);
        c.style.insertion = Some(name.to_owned());
        c.style.click = Some(("suggest_command".into(), format!("/msg {name} ")));
        c.style.hover = Some((
            "show_entity".into(),
            Box::new(Self::text(format!("{{name:\"{name}\",id:\"{uuid}\"}}"))),
        ));
        c
    }

    /// Parses chat JSON like vanilla's `IChatComponent.Serializer.jsonToComponent`.
    ///
    /// # Errors
    ///
    /// Vanilla's `JsonParseException` cases: invalid JSON, or JSON that is not a
    /// component.
    pub fn parse_json(text: &str) -> Result<Self, String> {
        let json = JsonValue::parse(text)?;
        Self::from_json(&json)
    }

    /// `IChatComponent.Serializer.deserialize`.
    ///
    /// # Errors
    ///
    /// JSON that does not describe a component.
    pub fn from_json(json: &JsonValue) -> Result<Self, String> {
        match json {
            JsonValue::Array(items) => {
                let mut items = items.iter();
                let mut first = Self::from_json(items.next().ok_or("empty component array")?)?;
                for item in items {
                    first.extra.push(Self::from_json(item)?);
                }
                Ok(first)
            }
            JsonValue::Object(_) => {
                let get = |k: &str| json.get(k);
                let content = if let Some(t) = get("text") {
                    Content::Text(json_as_string(t)?)
                } else if let Some(key) = get("translate") {
                    let key = json_as_string(key)?;
                    let mut with = Vec::new();
                    if let Some(args) = get("with") {
                        let JsonValue::Array(args) = args else {
                            return Err("'with' is not an array".into());
                        };
                        for a in args {
                            let c = Self::from_json(a)?;
                            with.push(match c {
                                Self {
                                    content: Content::Text(t),
                                    ref style,
                                    ref extra,
                                } if style.is_empty() && extra.is_empty() => Arg::Str(t),
                                c => Arg::Component(c),
                            });
                        }
                    }
                    Content::Translate { key, with }
                } else if let Some(score) = get("score") {
                    let (Some(name), Some(objective)) = (score.get("name"), score.get("objective"))
                    else {
                        return Err(
                            "A score component needs a least a name and an objective".into()
                        );
                    };
                    Content::Score {
                        name: json_as_string(name)?,
                        objective: json_as_string(objective)?,
                        value: match score.get("value") {
                            Some(v) => json_as_string(v)?,
                            None => String::new(),
                        },
                    }
                } else if let Some(selector) = get("selector") {
                    Content::Selector(json_as_string(selector)?)
                } else {
                    return Err(format!(
                        "Don't know how to turn {} into a Component",
                        json.to_gson_string()
                    ));
                };
                let mut extra = Vec::new();
                if let Some(e) = get("extra") {
                    let JsonValue::Array(items) = e else {
                        return Err("'extra' is not an array".into());
                    };
                    if items.is_empty() {
                        return Err("Unexpected empty array of components".into());
                    }
                    for item in items {
                        extra.push(Self::from_json(item)?);
                    }
                }
                Ok(Self {
                    content,
                    style: Style::from_json(json)?,
                    extra,
                })
            }
            JsonValue::Null => Err("Don't know how to turn null into a Component".into()),
            primitive => Ok(Self::text(json_as_string(primitive)?)),
        }
    }

    fn to_json(&self) -> JsonValue {
        if let Content::Text(t) = &self.content
            && self.style.is_empty()
            && self.extra.is_empty()
        {
            return JsonValue::from(t.as_str());
        }
        let mut o = JsonValue::object();
        let s = &self.style;
        for (key, value) in [
            ("bold", s.bold),
            ("italic", s.italic),
            ("underlined", s.underlined),
            ("strikethrough", s.strikethrough),
            ("obfuscated", s.obfuscated),
        ] {
            if let Some(v) = value {
                o = o.with(key, v);
            }
        }
        if let Some(c) = s.color {
            o = o.with("color", c);
        }
        if let Some(i) = &s.insertion {
            o = o.with("insertion", i.as_str());
        }
        if let Some((action, value)) = &s.click {
            o = o.with(
                "clickEvent",
                JsonValue::object()
                    .with("action", action.as_str())
                    .with("value", value.as_str()),
            );
        }
        if let Some((action, value)) = &s.hover {
            o = o.with(
                "hoverEvent",
                JsonValue::object()
                    .with("action", action.as_str())
                    .with("value", value.to_json()),
            );
        }
        if !self.extra.is_empty() {
            o = o.with(
                "extra",
                self.extra
                    .iter()
                    .map(Component::to_json)
                    .collect::<Vec<_>>(),
            );
        }
        match &self.content {
            Content::Text(t) => o.with("text", t.as_str()),
            Content::Score {
                name,
                objective,
                value,
            } => o.with(
                "score",
                JsonValue::object()
                    .with("name", name.as_str())
                    .with("objective", objective.as_str())
                    .with("value", value.as_str()),
            ),
            Content::Selector(s) => o.with("selector", s.as_str()),
            Content::Translate { key, with } => {
                let mut o = o.with("translate", key.as_str());
                if !with.is_empty() {
                    let args = with
                        .iter()
                        .map(|a| match a {
                            Arg::Component(c) => c.to_json(),
                            Arg::Str(s) => JsonValue::from(s.as_str()),
                        })
                        .collect::<Vec<_>>();
                    o = o.with("with", args);
                }
                o
            }
        }
    }

    /// Serializes like vanilla's chat Gson (HTML-safe escaping).
    pub fn to_json_string(&self) -> String {
        self.to_json().to_gson_string()
    }

    /// The plain text with translations resolved in en_US (vanilla:
    /// `getUnformattedText`), as written to the server log.
    pub fn unformatted(&self) -> String {
        let mut out = match &self.content {
            Content::Text(t) => t.clone(),
            Content::Score { value, .. } => value.clone(),
            Content::Selector(s) => s.clone(),
            Content::Translate { key, with } => {
                let args: Vec<String> = with
                    .iter()
                    .map(|a| match a {
                        Arg::Component(c) => c.unformatted(),
                        Arg::Str(s) => s.clone(),
                    })
                    .collect();
                format_translation(key, &args)
            }
        };
        for e in &self.extra {
            out.push_str(&e.unformatted());
        }
        out
    }

    /// Java's `toString()` of the component, which vanilla prints for disconnect
    /// reasons ("... lost connection: TextComponent{...}"). Only unstyled components are
    /// rendered exactly.
    pub fn java_to_string(&self) -> String {
        const STYLE: &str = "Style{hasParent=false, color=null, bold=null, italic=null, underlined=null, obfuscated=null, clickEvent=null, hoverEvent=null, insertion=null}";
        let siblings: Vec<String> = self.extra.iter().map(Component::java_to_string).collect();
        match &self.content {
            Content::Text(t) => format!(
                "TextComponent{{text='{t}', siblings=[{}], style={STYLE}}}",
                siblings.join(", ")
            ),
            Content::Score {
                name, objective, ..
            } => format!(
                "ScoreComponent{{name='{name}', objective='{objective}', siblings=[{}], style={STYLE}}}",
                siblings.join(", ")
            ),
            Content::Selector(s) => format!(
                "SelectorComponent{{pattern='{s}', siblings=[{}], style={STYLE}}}",
                siblings.join(", ")
            ),
            Content::Translate { key, with } => {
                let args: Vec<String> = with
                    .iter()
                    .map(|a| match a {
                        Arg::Component(c) => c.java_to_string(),
                        Arg::Str(s) => s.clone(),
                    })
                    .collect();
                format!(
                    "TranslatableComponent{{key='{key}', args=[{}], siblings=[{}], style={STYLE}}}",
                    args.join(", "),
                    siblings.join(", ")
                )
            }
        }
    }
}

/// en_US rendering of a translation key (see [`crate::lang`]).
fn format_translation(key: &str, args: &[String]) -> String {
    crate::lang::format(key, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTCH_UUID: &str = "b50ad385-829d-3141-a216-7e7d7539ba7f";

    /// Parsing then serializing normalizes like vanilla's Gson round trip.
    #[test]
    fn parses_like_vanilla() {
        let round = |json: &str| Component::parse_json(json).map(|c| c.to_json_string());
        assert_eq!(round("\"\"").as_deref(), Ok("\"\""));
        assert_eq!(round(r#"{"text":"hi"}"#).as_deref(), Ok("\"hi\""));
        assert_eq!(round("42").as_deref(), Ok("\"42\""));
        assert_eq!(
            round(r#"["a",{"text":"b","bold":true}]"#).as_deref(),
            Ok(r#"{"extra":[{"bold":true,"text":"b"}],"text":"a"}"#)
        );
        assert_eq!(
            round(r#"{"translate":"x","with":[{"text":"p"},"q"],"color":"RED"}"#).as_deref(),
            Ok(r#"{"color":"red","translate":"x","with":["p","q"]}"#)
        );
        assert!(Component::parse_json(r#"{"extra":[]}"#).is_err());
        assert!(Component::parse_json(r#"{"text":"a","extra":[]}"#).is_err());
        assert!(Component::parse_json("not json").is_err());
        assert_eq!(
            strip_formatting("\u{a7}cRed\u{a7}r \u{a7}zX"),
            "Red \u{a7}zX"
        );
    }

    /// The chat message vanilla 1.8.8 sent in parity/golden/play/join_flat.json.
    #[test]
    fn chat_message_matches_vanilla() {
        let msg = Component::translate(
            "chat.type.text",
            vec![
                Arg::Component(Component::player_name("Notch", NOTCH_UUID)),
                Arg::Str("hello world".into()),
            ],
        );
        assert_eq!(
            msg.to_json_string(),
            r#"{"translate":"chat.type.text","with":[{"insertion":"Notch","clickEvent":{"action":"suggest_command","value":"/msg Notch "},"hoverEvent":{"action":"show_entity","value":"{name:\"Notch\",id:\"b50ad385-829d-3141-a216-7e7d7539ba7f\"}"},"text":"Notch"},"hello world"]}"#
        );
        assert_eq!(msg.unformatted(), "<Notch> hello world");
    }

    #[test]
    fn join_message_and_to_string() {
        let joined = Component::translate(
            "multiplayer.player.joined",
            vec![Arg::Component(Component::player_name("Notch", NOTCH_UUID))],
        )
        .color("yellow");
        assert!(
            joined.to_json_string().starts_with(
                r#"{"color":"yellow","translate":"multiplayer.player.joined","with":[{"#
            )
        );
        assert_eq!(joined.unformatted(), "Notch joined the game");
        assert_eq!(
            Component::text("Disconnected").java_to_string(),
            "TextComponent{text='Disconnected', siblings=[], style=Style{hasParent=false, color=null, bold=null, italic=null, underlined=null, obfuscated=null, clickEvent=null, hoverEvent=null, insertion=null}}"
        );
        assert_eq!(Component::text("plain").to_json_string(), "\"plain\"");
    }
}
