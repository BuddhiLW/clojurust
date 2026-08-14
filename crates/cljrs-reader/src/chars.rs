//! Character classification for the reader.
//!
//! The bottom stratum of the crate: pure `char` predicates with no lexer or
//! parser state, so both the tokenizer and the pure rewrites can use them.

/// Returns `true` if `ch` is a valid constituent character for a symbol or
/// keyword.  Defined *negatively*: everything that isn't a delimiter, whitespace,
/// or special syntax character is a symbol constituent.
///
/// `#` is included here: it is a *non-terminating* macro character in the
/// Clojure reader, meaning it doesn't end a symbol token that's already in
/// progress (only a leading `#` triggers `#`-dispatch — see
/// [`is_symbol_start`]). This is what makes auto-gensym symbols like `x#`
/// tokenize as a single symbol rather than `x` followed by a stray `#`.
///
/// `:` is included here too — it is also non-terminating, so a keyword like
/// `:xlink:href` reads as one token with the literal name `xlink:href`
/// rather than splitting into two keywords at the embedded colon. Only a
/// *leading* `:`/`::` is special (it triggers keyword dispatch — see
/// [`is_symbol_start`]).
pub(crate) fn is_symbol_char(ch: char) -> bool {
    !matches!(
        ch,
        ' ' | '\t'
            | '\n'
            | '\r'
            | ','
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '"'
            | ';'
            | '`'
            | '~'
            | '^'
            | '@'
            | '\\'
    )
}

/// Returns `true` if `ch` can *start* a symbol (not a digit, not `#` since a
/// leading `#` always triggers `#`-dispatch, not `:` since a leading `:`
/// always triggers keyword dispatch, not `+`/`-` when the following char is
/// a digit — but the caller handles the `+`/`-` case).
pub(crate) fn is_symbol_start(ch: char) -> bool {
    is_symbol_char(ch) && !ch.is_ascii_digit() && ch != '#' && ch != ':'
}
