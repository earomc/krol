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
