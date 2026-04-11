//! Profile-to-prompt generation.
//!
//! Converts a `UserProfile` into system prompt sections that guide Tem's
//! communication style. The Firewall Rule: shapes COMMUNICATION only,
//! never work quality.

use crate::ethics::{above_threshold, CONFIDENCE_COSMETIC, CONFIDENCE_TONAL};
use crate::types::{RelationshipPhase, TraitScore, UserProfile};

/// Generate a user context section for the runtime system prompt.
///
/// Produces ~100-200 tokens of actionable guidance. Only includes dimensions
/// where confidence exceeds the cosmetic threshold (0.3).
pub fn section_user_profile(profile: &UserProfile) -> String {
    let mut lines = Vec::new();

    // Communication style dimensions
    let mut comm_parts = Vec::new();
    if let Some(ref d) = profile.communication_style.directness {
        if d.confidence >= CONFIDENCE_COSMETIC {
            comm_parts.push(format_dimension("direct", "indirect", d));
        }
    }
    if let Some(ref f) = profile.communication_style.formality {
        if f.confidence >= CONFIDENCE_COSMETIC {
            comm_parts.push(format_dimension("formal", "informal", f));
        }
    }
    if let Some(ref t) = profile.communication_style.technical_depth {
        if t.confidence >= CONFIDENCE_COSMETIC {
            comm_parts.push(format_dimension("highly technical", "non-technical", t));
        }
    }
    if let Some(ref v) = profile.communication_style.verbosity {
        if v.confidence >= CONFIDENCE_COSMETIC {
            comm_parts.push(format_dimension("verbose", "concise", v));
        }
    }
    if let Some(ref a) = profile.communication_style.analytical_vs_emotional {
        if a.confidence >= CONFIDENCE_COSMETIC {
            comm_parts.push(format_dimension("analytical", "emotional", a));
        }
    }

    if !comm_parts.is_empty() {
        lines.push(format!("- Communication: {}", comm_parts.join(", ")));
    }

    // Emotional state
    let mut mood_parts = Vec::new();
    if let Some(ref mood) = profile.emotional_state.current_mood {
        if profile.emotional_state.confidence >= CONFIDENCE_COSMETIC {
            mood_parts.push(mood.clone());
        }
    }
    if profile.emotional_state.stress_level > 0.6 {
        mood_parts.push("elevated stress".to_string());
    } else if profile.emotional_state.stress_level < 0.3
        && profile.emotional_state.confidence >= CONFIDENCE_COSMETIC
    {
        mood_parts.push("low stress".to_string());
    }
    if !mood_parts.is_empty() {
        lines.push(format!("- Mood: {}", mood_parts.join(", ")));
    }

    // Relationship phase
    lines.push(format!(
        "- Relationship: {} phase",
        phase_label(&profile.relationship_phase)
    ));

    // Recommendations (adapt line)
    if !profile.recommendations.adapt.is_empty() {
        lines.push(format!("- Adapt: {}", profile.recommendations.adapt));
    }
    if !profile.recommendations.avoid.is_empty() {
        lines.push(format!("- Avoid: {}", profile.recommendations.avoid));
    }

    if lines.is_empty() {
        return String::new();
    }

    let mut section = String::from("USER CONTEXT:\n");
    for line in &lines {
        section.push_str(line);
        section.push('\n');
    }
    section
}

/// Generate a shorter profile summary for the classifier (~50-100 tokens).
pub fn classifier_profile_summary(profile: &UserProfile) -> String {
    let mut parts = Vec::new();

    // Key communication traits only (above tonal threshold)
    if let Some(d) = profile.communication_style.directness.as_ref() {
        if above_threshold(&profile.communication_style.directness, CONFIDENCE_TONAL)
            && d.value > 0.6
        {
            parts.push("direct");
        } else if above_threshold(&profile.communication_style.directness, CONFIDENCE_TONAL)
            && d.value < 0.4
        {
            parts.push("indirect");
        }
    }
    if let Some(t) = profile.communication_style.technical_depth.as_ref() {
        if above_threshold(
            &profile.communication_style.technical_depth,
            CONFIDENCE_TONAL,
        ) && t.value > 0.6
        {
            parts.push("technical");
        }
    }
    if let Some(f) = profile.communication_style.formality.as_ref() {
        if above_threshold(&profile.communication_style.formality, CONFIDENCE_TONAL)
            && f.value > 0.6
        {
            parts.push("formal");
        } else if above_threshold(&profile.communication_style.formality, CONFIDENCE_TONAL)
            && f.value < 0.4
        {
            parts.push("casual");
        }
    }

    // Relationship phase
    let phase = phase_label(&profile.relationship_phase);

    if parts.is_empty() {
        format!("User: {} phase", phase)
    } else {
        format!("User: {} | {} phase", parts.join(", "), phase)
    }
}

/// Format a trait dimension as a human-readable label with confidence.
fn format_dimension(high_label: &str, low_label: &str, score: &TraitScore) -> String {
    let label = if score.value > 0.6 {
        high_label
    } else if score.value < 0.4 {
        low_label
    } else {
        return format!(
            "moderate {}/{} ({:.1})",
            high_label, low_label, score.confidence
        );
    };
    format!("{} ({:.1})", label, score.confidence)
}

/// Human-readable relationship phase label.
fn phase_label(phase: &RelationshipPhase) -> &'static str {
    match phase {
        RelationshipPhase::Discovery => "discovery",
        RelationshipPhase::Calibration => "calibration",
        RelationshipPhase::Partnership => "partnership",
        RelationshipPhase::DeepPartnership => "deep partnership",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CommunicationStyle, Recommendations, UserEmotionalState, UserProfile};

    fn make_score(value: f32, confidence: f32) -> Option<TraitScore> {
        Some(TraitScore {
            value,
            confidence,
            observations: 3,
            last_updated: 1000,
            reasoning: String::new(),
        })
    }

    #[test]
    fn empty_profile_minimal_output() {
        let profile = UserProfile::default();
        let section = section_user_profile(&profile);
        // Should at least have relationship phase
        assert!(section.contains("USER CONTEXT:"));
        assert!(section.contains("discovery phase"));
    }

    #[test]
    fn populated_profile_section() {
        let profile = UserProfile {
            user_id: "user_1".to_string(),
            communication_style: CommunicationStyle {
                directness: make_score(0.8, 0.6),
                formality: make_score(0.3, 0.5),
                technical_depth: make_score(0.9, 0.8),
                ..CommunicationStyle::default()
            },
            emotional_state: UserEmotionalState {
                current_mood: Some("engaged".to_string()),
                confidence: 0.7,
                reasoning: String::new(),
                stress_level: 0.1,
                energy_level: 0.8,
            },
            relationship_phase: RelationshipPhase::Calibration,
            recommendations: Recommendations {
                tone: String::new(),
                adapt: "Be concise, technical. Skip preamble.".to_string(),
                avoid: "lengthy explanations".to_string(),
            },
            ..UserProfile::default()
        };

        let section = section_user_profile(&profile);
        assert!(section.contains("Communication:"));
        assert!(section.contains("direct"));
        assert!(section.contains("informal"));
        assert!(section.contains("highly technical"));
        assert!(section.contains("Mood: engaged"));
        assert!(section.contains("low stress"));
        assert!(section.contains("calibration phase"));
        assert!(section.contains("Adapt: Be concise"));
        assert!(section.contains("Avoid: lengthy"));
    }

    #[test]
    fn low_confidence_dimensions_excluded() {
        let profile = UserProfile {
            user_id: "user_1".to_string(),
            communication_style: CommunicationStyle {
                directness: make_score(0.8, 0.2), // Below 0.3 threshold
                formality: make_score(0.3, 0.5),  // Above threshold
                ..CommunicationStyle::default()
            },
            ..UserProfile::default()
        };

        let section = section_user_profile(&profile);
        // directness should be excluded (confidence 0.2 < 0.3)
        assert!(!section.contains("direct ("));
        // formality should be included
        assert!(section.contains("informal"));
    }

    #[test]
    fn classifier_summary_basic() {
        let profile = UserProfile {
            user_id: "user_1".to_string(),
            communication_style: CommunicationStyle {
                directness: make_score(0.8, 0.6),
                technical_depth: make_score(0.9, 0.7),
                ..CommunicationStyle::default()
            },
            relationship_phase: RelationshipPhase::Partnership,
            ..UserProfile::default()
        };

        let summary = classifier_profile_summary(&profile);
        assert!(summary.contains("direct"));
        assert!(summary.contains("technical"));
        assert!(summary.contains("partnership phase"));
    }

    #[test]
    fn classifier_summary_empty_profile() {
        let profile = UserProfile::default();
        let summary = classifier_profile_summary(&profile);
        assert!(summary.contains("discovery phase"));
    }

    #[test]
    fn format_dimension_high() {
        let score = TraitScore {
            value: 0.8,
            confidence: 0.6,
            observations: 3,
            last_updated: 0,
            reasoning: String::new(),
        };
        let s = format_dimension("direct", "indirect", &score);
        assert_eq!(s, "direct (0.6)");
    }

    #[test]
    fn format_dimension_low() {
        let score = TraitScore {
            value: 0.2,
            confidence: 0.5,
            observations: 2,
            last_updated: 0,
            reasoning: String::new(),
        };
        let s = format_dimension("direct", "indirect", &score);
        assert_eq!(s, "indirect (0.5)");
    }

    #[test]
    fn format_dimension_moderate() {
        let score = TraitScore {
            value: 0.5,
            confidence: 0.4,
            observations: 2,
            last_updated: 0,
            reasoning: String::new(),
        };
        let s = format_dimension("direct", "indirect", &score);
        assert!(s.contains("moderate"));
    }
}
