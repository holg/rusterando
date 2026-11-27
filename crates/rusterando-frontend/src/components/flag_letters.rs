//! The shop name as the storefront banner paints it: whole letters in the
//! Italian flag's colours, per word the first third green, the middle white,
//! the rest red — "MUSTER" → MU·ST·ER, "PIZZERIA" → PI·ZZE·RIA, as on
//! a typical Italian shop sign. A pure function of the name, so SSR and hydration agree.

use leptos::prelude::*;

/// The colour class of each of a word's `n` letters: green for the first
/// `n / 3`, white for half the rest (rounded down), red for the others.
pub fn thirds(n: usize) -> Vec<&'static str> {
    let green = n / 3;
    let white = (n - green) / 2;
    (0..n)
        .map(|i| {
            if i < green {
                "fg"
            } else if i < green + white {
                "fw"
            } else {
                "fr"
            }
        })
        .collect()
}

/// `name` letter by letter in spans (`fg` / `fw` / `fr`), words kept apart.
pub fn flag_letters(name: &str) -> impl IntoView {
    let words: Vec<Vec<(char, &'static str)>> = name
        .split_whitespace()
        .map(|w| {
            let letters: Vec<char> = w.chars().collect();
            letters.iter().copied().zip(thirds(letters.len())).collect()
        })
        .collect();
    let last = words.len().saturating_sub(1);
    words
        .into_iter()
        .enumerate()
        .map(|(i, letters)| {
            view! {
                <span class="flag-word">
                    {letters.into_iter().map(|(c, cls)| view! { <span class=cls>{c.to_string()}</span> }).collect_view()}
                </span>
                {(i < last).then_some(" ")}
            }
        })
        .collect_view()
}

#[cfg(test)]
mod tests {
    use super::thirds;

    #[test]
    fn the_sign_s_split() {
        let s = |n| thirds(n).join(" ");
        // 6 / 7 / 8 letters (MUSTER / MUSTERS / PIZZERIA) as on a banner.
        assert_eq!(s(6), "fg fg fw fw fr fr");
        assert_eq!(s(7), "fg fg fw fw fr fr fr");
        assert_eq!(s(8), "fg fg fw fw fw fr fr fr");
        assert_eq!(s(3), "fg fw fr");
        assert_eq!(s(1), "fr");
        assert!(thirds(0).is_empty());
    }
}
