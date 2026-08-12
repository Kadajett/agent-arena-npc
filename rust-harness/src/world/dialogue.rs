use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::mcp::observation::{ChatLine, Observation};

const FEELING_PINGS: [&str; 14] = [
    "🙂", "😄", "🤔", "🧐", "😒", "😠", "😨", "😰", "😢", "😔", "😪", "😕", "🥱", "🤞",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DialogueChannel {
    Scene,
    Global,
    Private,
    Team,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DialogueKind {
    Speech,
    Melody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DialogueLine {
    pub channel: DialogueChannel,
    pub kind: DialogueKind,
    pub backend_message_type: Option<u8>,
    pub from: String,
    pub message: String,
    pub received_at: Option<DateTime<Utc>>,
}

pub struct DialogueWindow {
    pub lines: Vec<DialogueLine>,
    pub filtered_count: usize,
}

pub fn normalize_dialogue(observation: &Observation) -> DialogueWindow {
    // Compatibility releases have exposed the rolling buffer as either
    // `chat` or `recentChat`; during a transition both may be present. Never
    // choose one field and silently lose messages from the other channel.
    let mut source = Vec::with_capacity(observation.chat.len() + observation.recent_chat.len());
    for line in observation
        .chat
        .iter()
        .chain(observation.recent_chat.iter())
    {
        if !source.contains(line) {
            source.push(line.clone());
        }
    }
    let mut filtered_count = 0;
    let lines = source
        .iter()
        .filter_map(|line| {
            if let Some(line) = normalize_line(line) {
                Some(line)
            } else {
                filtered_count += 1;
                None
            }
        })
        .collect();
    DialogueWindow {
        lines,
        filtered_count,
    }
}

pub fn new_dialogue_lines(
    previous: &[DialogueLine],
    current: &[DialogueLine],
) -> Vec<DialogueLine> {
    let maximum_overlap = previous.len().min(current.len());
    let overlap = (0..=maximum_overlap)
        .rev()
        .find(|count| previous[previous.len() - count..] == current[..*count])
        .unwrap_or(0);
    current[overlap..].to_vec()
}

fn normalize_line(line: &ChatLine) -> Option<DialogueLine> {
    let message = line.message.as_deref()?.trim();
    if message.is_empty() || is_engine_chatter(message) || is_feeling_ping(message) {
        return None;
    }
    Some(DialogueLine {
        channel: dialogue_channel(line),
        kind: if is_melody(message) {
            DialogueKind::Melody
        } else {
            DialogueKind::Speech
        },
        backend_message_type: line.message_type,
        from: line
            .from
            .as_deref()
            .map(str::trim)
            .filter(|from| !from.is_empty())
            .unwrap_or("someone")
            .to_owned(),
        message: message.to_owned(),
        received_at: line
            .received_at
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc)),
    })
}

fn is_melody(message: &str) -> bool {
    message.starts_with("🎵 plays the ") && message.ends_with('♪')
}

fn dialogue_channel(line: &ChatLine) -> DialogueChannel {
    match line.message_type {
        Some(1) => DialogueChannel::Scene,
        Some(4) => DialogueChannel::Private,
        Some(8) => DialogueChannel::Team,
        Some(9) => DialogueChannel::Global,
        _ => match line.channel.as_deref() {
            Some("scene" | "room") => DialogueChannel::Scene,
            Some("global") => DialogueChannel::Global,
            Some("private") => DialogueChannel::Private,
            Some("team" | "group") => DialogueChannel::Team,
            _ => DialogueChannel::Unknown,
        },
    }
}

fn is_feeling_ping(message: &str) -> bool {
    FEELING_PINGS.contains(&message.trim())
}

fn is_engine_chatter(message: &str) -> bool {
    let candidate = message.trim().trim_matches('"');
    let Some((namespace, event)) = candidate.split_once('.') else {
        return false;
    };
    !namespace.is_empty()
        && namespace
            .chars()
            .all(|character| character.is_ascii_lowercase())
        && !event.is_empty()
        && event
            .chars()
            .all(|character| character.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(from: &str, message: &str, received_at: &str) -> ChatLine {
        ChatLine {
            channel: None,
            message_type: None,
            from: Some(from.to_owned()),
            message: Some(message.to_owned()),
            received_at: Some(received_at.to_owned()),
        }
    }

    #[test]
    fn filters_engine_keys_and_feelings_without_hiding_player_speech() {
        let observation = Observation {
            recent_chat: vec![
                line("", "chat.joinedRoom", "2026-08-11T12:00:00Z"),
                line("Cassian", "🤔", "2026-08-11T12:00:01Z"),
                line(
                    "Mara",
                    "chat.joinedRoom is a strange thing to say.",
                    "2026-08-11T12:00:02Z",
                ),
            ],
            ..Observation::default()
        };

        let window = normalize_dialogue(&observation);

        assert_eq!(window.filtered_count, 2);
        assert_eq!(window.lines.len(), 1);
        assert_eq!(window.lines[0].from, "Mara");
    }

    #[test]
    fn overlapping_windows_emit_only_new_lines() {
        let first = vec![
            DialogueLine {
                channel: DialogueChannel::Scene,
                kind: DialogueKind::Speech,
                backend_message_type: Some(1),
                from: "A".to_owned(),
                message: "one".to_owned(),
                received_at: None,
            },
            DialogueLine {
                channel: DialogueChannel::Scene,
                kind: DialogueKind::Speech,
                backend_message_type: Some(1),
                from: "B".to_owned(),
                message: "two".to_owned(),
                received_at: None,
            },
        ];
        let mut second = first.clone();
        second.push(DialogueLine {
            channel: DialogueChannel::Scene,
            kind: DialogueKind::Speech,
            backend_message_type: Some(1),
            from: "A".to_owned(),
            message: "one".to_owned(),
            received_at: None,
        });

        assert_eq!(new_dialogue_lines(&first, &second), second[2..]);
        assert!(new_dialogue_lines(&second, &second).is_empty());
    }

    #[test]
    fn backend_message_type_distinguishes_private_and_team_from_global_room() {
        let observation = Observation {
            recent_chat: vec![
                ChatLine {
                    channel: Some("global".to_owned()),
                    message_type: Some(4),
                    from: Some("A".to_owned()),
                    message: Some("private".to_owned()),
                    received_at: None,
                },
                ChatLine {
                    channel: Some("global".to_owned()),
                    message_type: Some(8),
                    from: Some("B".to_owned()),
                    message: Some("team".to_owned()),
                    received_at: None,
                },
                ChatLine {
                    channel: Some("global".to_owned()),
                    message_type: Some(9),
                    from: Some("C".to_owned()),
                    message: Some("global".to_owned()),
                    received_at: None,
                },
            ],
            ..Observation::default()
        };

        let channels = normalize_dialogue(&observation)
            .lines
            .into_iter()
            .map(|line| line.channel)
            .collect::<Vec<_>>();
        assert_eq!(
            channels,
            vec![
                DialogueChannel::Private,
                DialogueChannel::Team,
                DialogueChannel::Global
            ]
        );
    }

    #[test]
    fn backend_melody_stage_direction_is_not_normal_speech() {
        let observation = Observation {
            recent_chat: vec![ChatLine {
                channel: Some("scene".to_owned()),
                message_type: Some(1),
                from: Some("Cassian".to_owned()),
                message: Some("🎵 plays the lute: C E G C5 ♪".to_owned()),
                received_at: None,
            }],
            ..Observation::default()
        };

        let line = normalize_dialogue(&observation)
            .lines
            .into_iter()
            .next()
            .expect("melody line");
        assert_eq!(line.kind, DialogueKind::Melody);
        assert_eq!(line.channel, DialogueChannel::Scene);
    }

    #[test]
    fn merges_compatibility_chat_fields_without_dropping_channels_or_duplicates() {
        let shared = line("SceneAgent", "hello from the room", "2026-08-11T12:00:00Z");
        let observation = Observation {
            chat: vec![
                shared.clone(),
                line("GlobalAgent", "global hello", "2026-08-11T12:00:01Z"),
            ],
            recent_chat: vec![
                shared,
                line("PrivateAgent", "private hello", "2026-08-11T12:00:02Z"),
            ],
            ..Observation::default()
        };

        let window = normalize_dialogue(&observation);

        assert_eq!(window.filtered_count, 0);
        assert_eq!(window.lines.len(), 3);
        assert_eq!(window.lines[0].from, "SceneAgent");
        assert_eq!(window.lines[1].from, "GlobalAgent");
        assert_eq!(window.lines[2].from, "PrivateAgent");
    }
}
