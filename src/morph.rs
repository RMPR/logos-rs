//! Decoders that turn terse morphology codes into human-readable parsing.
//!
//! Two systems are supported:
//! - **OSHM** (OpenScriptures Hebrew Morphology), used by `morphhb`, e.g.
//!   `HVqp3ms` → "Qal perfect, 3rd person masculine singular".
//! - **MorphGNT** part-of-speech + 8-character parse code, used by SBLGNT,
//!   e.g. POS `N-`, parse `----GSF-` → "noun — genitive singular feminine".

// ---------------------------------------------------------------------------
// Hebrew (OSHM)
// ---------------------------------------------------------------------------

/// Decode a full OSHM morph string (may contain `/`-separated morphemes and a
/// leading language letter `H`/`A`). Returns a human-readable description.
pub fn decode_hebrew(code: &str) -> String {
    if code.is_empty() {
        return String::new();
    }
    code.split('/')
        .map(decode_hebrew_segment)
        .collect::<Vec<_>>()
        .join(" + ")
}

fn decode_hebrew_segment(seg: &str) -> String {
    let mut c = seg.chars().peekable();
    // optional language marker on the first segment
    let mut aramaic = false;
    if matches!(c.peek(), Some('H') | Some('A')) {
        aramaic = c.peek() == Some(&'A');
        c.next();
    }
    let rest: String = c.collect();
    let label = decode_hebrew_pos(&rest);
    if aramaic { format!("Aramaic {label}") } else { label }
}

fn hb_gender(ch: Option<char>) -> &'static str {
    match ch {
        Some('m') => "masculine",
        Some('f') => "feminine",
        Some('b') => "both",
        Some('c') => "common",
        _ => "",
    }
}
fn hb_number(ch: Option<char>) -> &'static str {
    match ch {
        Some('s') => "singular",
        Some('p') => "plural",
        Some('d') => "dual",
        _ => "",
    }
}
fn hb_state(ch: Option<char>) -> &'static str {
    match ch {
        Some('a') => "absolute",
        Some('c') => "construct",
        Some('d') => "determined",
        _ => "",
    }
}
fn hb_person(ch: Option<char>) -> &'static str {
    match ch {
        Some('1') => "1st person",
        Some('2') => "2nd person",
        Some('3') => "3rd person",
        _ => "",
    }
}

fn join(parts: &[&str]) -> String {
    parts.iter().filter(|p| !p.is_empty()).cloned().collect::<Vec<_>>().join(" ")
}

fn decode_hebrew_pos(s: &str) -> String {
    let b = s.as_bytes();
    let ch = |i: usize| s.chars().nth(i);
    match b.first().map(|&x| x as char) {
        Some('A') => {
            let kind = match ch(1) {
                Some('a') => "adjective",
                Some('c') => "cardinal number",
                Some('g') => "gentilic adjective",
                Some('o') => "ordinal number",
                _ => "adjective",
            };
            join(&[kind, hb_gender(ch(2)), hb_number(ch(3)), hb_state(ch(4))])
        }
        Some('C') => "conjunction".into(),
        Some('D') => "adverb".into(),
        Some('N') => match ch(1) {
            Some('p') => "proper noun".into(),
            Some('g') => join(&["gentilic noun", hb_gender(ch(2)), hb_number(ch(3)), hb_state(ch(4))]),
            _ => join(&["noun", hb_gender(ch(2)), hb_number(ch(3)), hb_state(ch(4))]),
        },
        Some('P') => {
            let kind = match ch(1) {
                Some('d') => "demonstrative pronoun",
                Some('f') => "indefinite pronoun",
                Some('i') => "interrogative pronoun",
                Some('p') => "personal pronoun",
                Some('r') => "relative pronoun",
                _ => "pronoun",
            };
            join(&[kind, hb_person(ch(2)), hb_gender(ch(3)), hb_number(ch(4))])
        }
        Some('R') => {
            if ch(1) == Some('d') { "preposition + article".into() } else { "preposition".into() }
        }
        Some('S') => match ch(1) {
            Some('p') => join(&["pronominal suffix", hb_person(ch(2)), hb_gender(ch(3)), hb_number(ch(4))]),
            Some('d') => "directional he".into(),
            Some('h') => "paragogic he".into(),
            Some('n') => "paragogic nun".into(),
            _ => "suffix".into(),
        },
        Some('T') => match ch(1) {
            Some('a') => "affirmation particle".into(),
            Some('d') => "definite article".into(),
            Some('e') => "exhortation particle".into(),
            Some('i') => "interrogative particle".into(),
            Some('j') => "interjection".into(),
            Some('m') => "demonstrative particle".into(),
            Some('n') => "negative particle".into(),
            Some('o') => "direct object marker".into(),
            Some('r') => "relative particle".into(),
            _ => "particle".into(),
        },
        Some('V') => decode_hebrew_verb(s),
        _ => s.to_string(),
    }
}

fn decode_hebrew_verb(s: &str) -> String {
    let ch = |i: usize| s.chars().nth(i);
    let stem = match ch(1) {
        Some('q') => "Qal",
        Some('N') => "Niphal",
        Some('p') => "Piel",
        Some('P') => "Pual",
        Some('h') => "Hiphil",
        Some('H') => "Hophal",
        Some('t') => "Hithpael",
        Some('o') => "Polel",
        Some('O') => "Polal",
        Some('r') => "Hithpolel",
        Some('e') => "Tiphil",
        _ => "",
    };
    let conj = ch(2);
    let aspect = match conj {
        Some('p') => "perfect",
        Some('q') => "sequential perfect",
        Some('i') => "imperfect",
        Some('w') => "sequential imperfect",
        Some('h') => "cohortative",
        Some('j') => "jussive",
        Some('v') => "imperative",
        Some('r') => "active participle",
        Some('s') => "passive participle",
        Some('a') => "infinitive absolute",
        Some('c') => "infinitive construct",
        _ => "",
    };
    // Participles inflect for gender/number/state; finite forms for person/gender/number.
    let tail = match conj {
        Some('r') | Some('s') => {
            join(&[hb_gender(ch(3)), hb_number(ch(4)), hb_state(ch(5))])
        }
        Some('a') | Some('c') => String::new(),
        _ => join(&[hb_person(ch(3)), hb_gender(ch(4)), hb_number(ch(5))]),
    };
    let head = join(&[stem, aspect]);
    if tail.is_empty() { head } else { format!("{head}, {tail}") }
}

// ---------------------------------------------------------------------------
// Greek (MorphGNT)
// ---------------------------------------------------------------------------

/// Decode a MorphGNT part-of-speech code (`N-`, `V-`, `RA`, …) plus its
/// 8-character parse string into a human-readable description.
pub fn decode_greek(pos: &str, parse: &str) -> String {
    let label = greek_pos(pos);
    let p: Vec<char> = parse.chars().collect();
    let at = |i: usize| p.get(i).copied().filter(|&c| c != '-');

    let person = match at(0) {
        Some('1') => "1st person",
        Some('2') => "2nd person",
        Some('3') => "3rd person",
        _ => "",
    };
    let tense = match at(1) {
        Some('P') => "present",
        Some('I') => "imperfect",
        Some('F') => "future",
        Some('A') => "aorist",
        Some('X') => "perfect",
        Some('Y') => "pluperfect",
        _ => "",
    };
    let voice = match at(2) {
        Some('A') => "active",
        Some('M') => "middle",
        Some('P') => "passive",
        _ => "",
    };
    let mood = match at(3) {
        Some('I') => "indicative",
        Some('D') => "imperative",
        Some('S') => "subjunctive",
        Some('O') => "optative",
        Some('N') => "infinitive",
        Some('P') => "participle",
        _ => "",
    };
    let case_ = match at(4) {
        Some('N') => "nominative",
        Some('G') => "genitive",
        Some('D') => "dative",
        Some('A') => "accusative",
        Some('V') => "vocative",
        _ => "",
    };
    let number = match at(5) {
        Some('S') => "singular",
        Some('P') => "plural",
        _ => "",
    };
    let gender = match at(6) {
        Some('M') => "masculine",
        Some('F') => "feminine",
        Some('N') => "neuter",
        _ => "",
    };
    let degree = match at(7) {
        Some('C') => "comparative",
        Some('S') => "superlative",
        _ => "",
    };

    // Order verbal features first, then nominal agreement.
    let features = join(&[tense, voice, mood, person, case_, number, gender, degree]);
    if features.is_empty() { label.to_string() } else { format!("{label} — {features}") }
}

fn greek_pos(pos: &str) -> &'static str {
    match pos {
        "N-" => "noun",
        "A-" => "adjective",
        "RA" => "article",
        "RD" => "demonstrative pronoun",
        "RI" => "interrogative/indefinite pronoun",
        "RP" => "personal pronoun",
        "RR" => "relative pronoun",
        "C-" => "conjunction",
        "D-" => "adverb",
        "I-" => "interjection",
        "P-" => "preposition",
        "V-" => "verb",
        "X-" => "particle",
        "M-" => "indeclinable",
        _ => "word",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hebrew() {
        assert_eq!(decode_hebrew("HVqp3ms"), "Qal perfect, 3rd person masculine singular");
        assert_eq!(decode_hebrew("HNcmsc"), "noun masculine singular construct");
        assert_eq!(decode_hebrew("HNp"), "proper noun");
        assert_eq!(decode_hebrew("HRd/Ncmpa"), "preposition + article + noun masculine plural absolute");
        assert_eq!(decode_hebrew("HC/Vqh1cp"), "conjunction + Qal cohortative, 1st person common plural");
        assert_eq!(decode_hebrew("HPp2ms"), "personal pronoun 2nd person masculine singular");
    }

    #[test]
    fn greek() {
        assert_eq!(decode_greek("N-", "----GSF-"), "noun — genitive singular feminine");
        assert_eq!(decode_greek("V-", "3AAI-S--"), "verb — aorist active indicative 3rd person singular");
        assert_eq!(decode_greek("RA", "----NSM-"), "article — nominative singular masculine");
        assert_eq!(decode_greek("C-", "--------"), "conjunction");
    }
}
