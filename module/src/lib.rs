//! VerdictLock: a URL_SCAN scoring module for Telegraph.
//!
//! Exports the node's ABI: `alloc`, `dealloc`, `rank_answer`, plus linear memory.
//! No imports, no allocator, no floats beyond f32 arithmetic, every loop bounded.

#![no_std]

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

// ---------------------------------------------------------------- allocator

const HEAP_SIZE: usize = 4 << 20;
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];
static mut HEAP_OFFSET: usize = 0;

#[no_mangle]
pub extern "C" fn alloc(size: i32) -> i32 {
    let size = if size < 0 { 0 } else { size as usize };
    unsafe {
        let base = core::ptr::addr_of_mut!(HEAP).cast::<u8>();
        let mut off = (HEAP_OFFSET + 7) & !7;
        if off + size > HEAP_SIZE {
            off = 0;
        }
        HEAP_OFFSET = off + size.min(HEAP_SIZE);
        base.add(off) as i32
    }
}

#[no_mangle]
pub extern "C" fn dealloc(_ptr: i32, _size: i32) {}

const MAX_BYTES: usize = 65536;

unsafe fn read_bytes<'a>(ptr: i32, len: i32) -> &'a [u8] {
    if ptr <= 0 || len <= 0 {
        return &[];
    }
    let len = (len as usize).min(MAX_BYTES);
    unsafe { core::slice::from_raw_parts(ptr as *const u8, len) }
}

// ------------------------------------------------------------------ tokens

const MAX_TOKENS: usize = 768;

struct Toks {
    start: [u32; MAX_TOKENS],
    len: [u16; MAX_TOKENS],
    hash: [u32; MAX_TOKENS],
    stem: [u32; MAX_TOKENS],
    stem_len: [u16; MAX_TOKENS],
    weight: [f32; MAX_TOKENS],
    numeric: [bool; MAX_TOKENS],
    /// set when a clause separator follows the token: `No,` opens an answer,
    /// `no` in the middle of one negates what comes next. Written on every push.
    bnd: [bool; MAX_TOKENS],
    count: usize,
}

const EMPTY_TOKS: Toks = Toks {
    start: [0; MAX_TOKENS],
    len: [0; MAX_TOKENS],
    hash: [0; MAX_TOKENS],
    stem: [0; MAX_TOKENS],
    stem_len: [0; MAX_TOKENS],
    weight: [0.0; MAX_TOKENS],
    numeric: [false; MAX_TOKENS],
    bnd: [false; MAX_TOKENS],
    count: 0,
};

static mut TOK_Q: Toks = EMPTY_TOKS;
static mut TOK_GT: Toks = EMPTY_TOKS;
static mut TOK_MA: Toks = EMPTY_TOKS;

#[inline]
fn lower(b: u8) -> u8 {
    if b.is_ascii_uppercase() {
        b + 32
    } else {
        b
    }
}

#[inline]
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b >= 0x80
}

/// A digit-group separator only counts as part of the token when digits sit on
/// both sides, so `0.95` and `104.21.7.19` survive but `example.com` splits.
#[inline]
fn joins_digits(text: &[u8], i: usize) -> bool {
    let b = text[i];
    if b != b'.' && b != b',' {
        return false;
    }
    i > 0 && text[i - 1].is_ascii_digit() && i + 1 < text.len() && text[i + 1].is_ascii_digit()
}

const STOPWORDS: [&[u8]; 62] = [
    b"the", b"a", b"an", b"is", b"are", b"was", b"were", b"be", b"been", b"being", b"and", b"or",
    b"of", b"to", b"in", b"on", b"at", b"by", b"for", b"with", b"as", b"it", b"its", b"this",
    b"that", b"these", b"those", b"from", b"has", b"have", b"had", b"do", b"does", b"did", b"you",
    b"your", b"we", b"our", b"they", b"their", b"i", b"me", b"my", b"so", b"if", b"then", b"than",
    b"there", b"here", b"about", b"any", b"all", b"can", b"could", b"would", b"should", b"may",
    b"might", b"will", b"shall", b"but", b"however",
];

const BOILERPLATE: [&[u8]; 24] = [
    b"sure", b"happy", b"help", b"hope", b"hopefully", b"certainly", b"absolutely", b"please",
    b"let", b"know", b"need", b"anything", b"else", b"based", b"analysis", b"conclusion",
    b"overall", b"summary", b"note", b"noting", b"worth", b"general", b"terms", b"regards",
];

fn eq_ci_across(left: &[u8], a: (usize, usize), right: &[u8], b: (usize, usize)) -> bool {
    if a.1 != b.1 {
        return false;
    }
    let mut i = 0;
    while i < a.1 {
        if lower(left[a.0 + i]) != lower(right[b.0 + i]) {
            return false;
        }
        i += 1;
    }
    true
}

/// Compares two figures ignoring thousands separators, so `299,792,458` and
/// `299792458` are the same number and `300,000` is not.
fn figures_equal(a_text: &[u8], a: (usize, usize), b_text: &[u8], b: (usize, usize)) -> bool {
    let (mut i, mut j) = (0usize, 0usize);
    loop {
        while i < a.1 && a_text[a.0 + i] == b',' {
            i += 1;
        }
        while j < b.1 && b_text[b.0 + j] == b',' {
            j += 1;
        }
        if i >= a.1 || j >= b.1 {
            return i >= a.1 && j >= b.1;
        }
        if a_text[a.0 + i] != b_text[b.0 + j] {
            return false;
        }
        i += 1;
        j += 1;
    }
}

const NUMBER_WORDS: [(&[u8], &[u8]); 28] = [
    (b"zero", b"0"), (b"one", b"1"), (b"two", b"2"), (b"three", b"3"), (b"four", b"4"),
    (b"five", b"5"), (b"six", b"6"), (b"seven", b"7"), (b"eight", b"8"), (b"nine", b"9"),
    (b"ten", b"10"), (b"eleven", b"11"), (b"twelve", b"12"), (b"thirteen", b"13"),
    (b"fourteen", b"14"), (b"fifteen", b"15"), (b"sixteen", b"16"), (b"seventeen", b"17"),
    (b"eighteen", b"18"), (b"nineteen", b"19"), (b"twenty", b"20"), (b"thirty", b"30"),
    (b"forty", b"40"), (b"fifty", b"50"), (b"sixty", b"60"), (b"seventy", b"70"),
    (b"eighty", b"80"), (b"ninety", b"90"),
];

/// The digits a spelled-out number stands for, if it is one.
fn number_word(text: &[u8], start: usize, len: usize) -> Option<&'static [u8]> {
    for (word, digits) in NUMBER_WORDS.iter() {
        if word.len() == len {
            let mut i = 0;
            let mut same = true;
            while i < len {
                if lower(text[start + i]) != word[i] {
                    same = false;
                    break;
                }
                i += 1;
            }
            if same {
                return Some(digits);
            }
        }
    }
    None
}

fn digits_equal(text: &[u8], span: (usize, usize), digits: &[u8]) -> bool {
    if span.1 != digits.len() {
        return false;
    }
    let mut i = 0;
    while i < digits.len() {
        if text[span.0 + i] != digits[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Words match on their stem: `engine`/`engines`, `flag`/`flagged`,
/// `verify`/`verified`. A miner that answers correctly in another tense has
/// answered correctly.
fn words_match(
    a_text: &[u8],
    a: &Toks,
    ai: usize,
    b_text: &[u8],
    b: &Toks,
    bi: usize,
) -> bool {
    let sa = (a.start[ai] as usize, a.len[ai] as usize);
    let sb = (b.start[bi] as usize, b.len[bi] as usize);
    if a.numeric[ai] != b.numeric[bi] {
        // one side spelled the figure out
        let (word_text, word, digit_text, digit) = if a.numeric[ai] {
            (b_text, sb, a_text, sa)
        } else {
            (a_text, sa, b_text, sb)
        };
        return match number_word(word_text, word.0, word.1) {
            Some(digits) => digits_equal(digit_text, digit, digits),
            None => false,
        };
    }
    if a.numeric[ai] {
        return figures_equal(a_text, sa, b_text, sb);
    }
    if a.hash[ai] == b.hash[bi] && eq_ci_across(a_text, sa, b_text, sb) {
        return true;
    }
    let (la, lb) = (a.stem_len[ai] as usize, b.stem_len[bi] as usize);
    if la != lb || la < 3 || a.stem[ai] != b.stem[bi] {
        return false;
    }
    let mut k = 0;
    while k < la {
        if lower(a_text[sa.0 + k]) != lower(b_text[sb.0 + k]) {
            return false;
        }
        k += 1;
    }
    true
}

#[inline]
fn is_vowel(b: u8) -> bool {
    matches!(b, b'a' | b'e' | b'i' | b'o' | b'u' | b'y')
}

fn ends_with(text: &[u8], start: usize, len: usize, suffix: &[u8]) -> bool {
    if len < suffix.len() {
        return false;
    }
    let mut i = 0;
    while i < suffix.len() {
        if lower(text[start + len - suffix.len() + i]) != suffix[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Enough stemming to see that `splice` and `splicing`, `engine` and `engines`,
/// `flag` and `flagged` are the same word. A miner that answers in another tense
/// has answered.
fn stem_length(text: &[u8], start: usize, len: usize) -> usize {
    let mut l = len;
    if l >= 6 && ends_with(text, start, l, b"ing") {
        l -= 3;
    } else if l >= 5 && ends_with(text, start, l, b"ly") {
        l -= 2;
    } else if l >= 5 && ends_with(text, start, l, b"ed") {
        l -= 2;
    } else if l >= 5 && ends_with(text, start, l, b"es") {
        l -= 2;
    } else if l >= 4 && ends_with(text, start, l, b"s") && !ends_with(text, start, l, b"ss") {
        l -= 1;
    }
    if l >= 4 {
        let last = lower(text[start + l - 1]);
        let prev = lower(text[start + l - 2]);
        if last == prev && !is_vowel(last) {
            l -= 1;
        }
    }
    if l >= 4 && lower(text[start + l - 1]) == b'e' {
        l -= 1;
    }
    l
}

fn token_hash(text: &[u8], start: usize, len: usize) -> u32 {
    let mut h: u32 = 2166136261;
    let mut i = 0;
    while i < len {
        h ^= lower(text[start + i]) as u32;
        h = h.wrapping_mul(16777619);
        i += 1;
    }
    h
}

fn matches_list(text: &[u8], start: usize, len: usize, list: &[&[u8]]) -> bool {
    for word in list {
        if word.len() == len {
            let mut i = 0;
            let mut same = true;
            while i < len {
                if lower(text[start + i]) != word[i] {
                    same = false;
                    break;
                }
                i += 1;
            }
            if same {
                return true;
            }
        }
    }
    false
}

/// What a token is worth as evidence that the answer was given. Figures and
/// names carry the answer; ordinary words carry the sentence around it. This is
/// a corpus-free stand-in for IDF, and it is what separates "same topic" from
/// "same answer".
/// Like `matches_list`, but a regular inflection of a lexicon word counts:
/// `climbed` is `climb`, `gains` is `gain`, `flagged` is `flag`. The accepted
/// endings are listed rather than stemmed, so `really` is not `real`.
fn matches_list_infl(text: &[u8], start: usize, len: usize, list: &[&[u8]]) -> bool {
    if matches_list(text, start, len, list) {
        return true;
    }
    for word in list {
        if len <= word.len() || len > word.len() + 4 {
            continue;
        }
        let mut i = 0;
        let mut same = true;
        while i < word.len() {
            if lower(text[start + i]) != word[i] {
                same = false;
                break;
            }
            i += 1;
        }
        if !same {
            continue;
        }
        let tail_start = start + word.len();
        let tail_len = len - word.len();
        let tail = |bytes: &[u8]| -> bool {
            if bytes.len() != tail_len {
                return false;
            }
            let mut k = 0;
            while k < tail_len {
                if lower(text[tail_start + k]) != bytes[k] {
                    return false;
                }
                k += 1;
            }
            true
        };
        if tail(b"s") || tail(b"es") || tail(b"ed") || tail(b"d") || tail(b"ing") {
            return true;
        }
        // a doubled final consonant: flag -> flagged, stop -> stopping
        let last = word[word.len() - 1];
        if tail_len >= 3 && lower(text[tail_start]) == last && !is_vowel(last) {
            let mut k = 1;
            let mut rest = [0u8; 3];
            while k < tail_len && k <= 3 {
                rest[k - 1] = lower(text[tail_start + k]);
                k += 1;
            }
            if (tail_len == 3 && rest[0] == b'e' && rest[1] == b'd')
                || (tail_len == 4 && rest[0] == b'i' && rest[1] == b'n' && rest[2] == b'g')
            {
                return true;
            }
        }
    }
    false
}

fn weigh(text: &[u8], start: usize, len: usize, numeric: bool, opens_sentence: bool) -> f32 {
    if numeric {
        return 3.0;
    }
    if matches_list(text, start, len, &STOPWORDS) {
        return 0.08;
    }
    if matches_list(text, start, len, &BOILERPLATE) {
        return 0.05;
    }
    let mut has_alpha = false;
    let mut has_digit = false;
    let mut all_upper = len > 1;
    let mut i = 0;
    while i < len {
        let b = text[start + i];
        if b.is_ascii_alphabetic() {
            has_alpha = true;
            if !b.is_ascii_uppercase() {
                all_upper = false;
            }
        } else if b.is_ascii_digit() {
            has_digit = true;
        } else {
            all_upper = false;
        }
        i += 1;
    }
    if has_alpha && has_digit {
        return 3.0; // an identifier: EIP4844, IPv6, SHA256
    }
    if all_upper && len <= 6 {
        return 2.8; // an abbreviation the answer turns on
    }
    if !opens_sentence && text[start].is_ascii_uppercase() && len >= 3 {
        return 2.8; // a name
    }
    match len {
        0..=2 => 0.25,
        3..=4 => 1.2,
        5..=7 => 1.6,
        _ => 2.2,
    }
}

fn tokenize(text: &[u8], toks: &mut Toks) {
    toks.count = 0;
    let mut i = 0;
    while i < text.len() && toks.count < MAX_TOKENS {
        if !is_word_byte(text[i]) {
            i += 1;
            continue;
        }
        let start = i;
        let mut digits = 0usize;
        let mut alpha = 0usize;
        #[allow(unused_assignments)]
        while i < text.len() && (is_word_byte(text[i]) || joins_digits(text, i)) {
            if text[i].is_ascii_digit() {
                digits += 1;
            } else if text[i].is_ascii_alphabetic() {
                alpha += 1;
            }
            i += 1;
        }
        let mut len = i - start;
        // `$3.1T`, `11.2B`, `95%`: the figure and its unit are two tokens
        if digits > 0 && alpha > 0 && alpha <= 3 {
            let mut split = start;
            while split < i && (text[split].is_ascii_digit() || joins_digits(text, split)) {
                split += 1;
            }
            if split > start && split < i && i - split == alpha {
                len = split - start;
                i = split;
                alpha = 0;
            }
        }
        let numeric = digits > 0 && alpha == 0;
        let n = toks.count;
        toks.start[n] = start as u32;
        toks.len[n] = len.min(u16::MAX as usize) as u16;
        toks.hash[n] = token_hash(text, start, len);
        let stem_len = if numeric { len } else { stem_length(text, start, len) };
        toks.stem_len[n] = stem_len as u16;
        toks.stem[n] = token_hash(text, start, stem_len);
        let opens_sentence = n == 0 || toks.bnd[n - 1];
        toks.weight[n] = weigh(text, start, len, numeric, opens_sentence);
        toks.numeric[n] = numeric;
        toks.bnd[n] = i < text.len() && matches!(text[i], b',' | b';' | b'.' | b':' | b'!' | b'?');
        if i < text.len() && text[i] == b'(' && !numeric {
            toks.weight[n] = 3.0; // `len()` is the answer, `call` is the sentence
        }
        toks.count = n + 1;
    }
}

// ---------------------------------------------------------------- trigrams

const TRI_WORDS: usize = 1024;
static mut TRI_GT: [u64; TRI_WORDS] = [0; TRI_WORDS];
static mut TRI_MA: [u64; TRI_WORDS] = [0; TRI_WORDS];

/// Sets the trigram bits of the token stream, normalised to single spaces so
/// punctuation and spacing cannot move the score.
fn trigram_bits(text: &[u8], toks: &Toks, bits: &mut [u64; TRI_WORDS]) -> u32 {
    for slot in bits.iter_mut() {
        *slot = 0;
    }
    let mut window = [0u8; 3];
    let mut filled = 0usize;
    let mut count = 0u32;
    let mut t = 0usize;
    while t < toks.count {
        let start = toks.start[t] as usize;
        let len = toks.len[t] as usize;
        let mut i = 0;
        while i <= len {
            let byte = if i == len { b' ' } else { lower(text[start + i]) };
            window[0] = window[1];
            window[1] = window[2];
            window[2] = byte;
            if filled < 3 {
                filled += 1;
            }
            if filled == 3 {
                let mut h: u32 = 2166136261;
                for b in window.iter() {
                    h ^= *b as u32;
                    h = h.wrapping_mul(16777619);
                }
                let index = (h & 0xFFFF) as usize;
                let word = index >> 6;
                let bit = 1u64 << (index & 63);
                if bits[word] & bit == 0 {
                    bits[word] |= bit;
                    count += 1;
                }
            }
            i += 1;
        }
        t += 1;
    }
    count
}

fn popcount_and(a: &[u64; TRI_WORDS], b: &[u64; TRI_WORDS]) -> u32 {
    let mut total = 0u32;
    let mut i = 0;
    while i < TRI_WORDS {
        total += (a[i] & b[i]).count_ones();
        i += 1;
    }
    total
}

// ---------------------------------------------------------------- entities

const MAX_ENTITIES: usize = 12;
const ENT_BUF: usize = 4096;

struct Ents {
    buf: [u8; ENT_BUF],
    start: [u16; MAX_ENTITIES],
    len: [u16; MAX_ENTITIES],
    kind: [u8; MAX_ENTITIES],
    count: usize,
    used: usize,
}

const EMPTY_ENTS: Ents = Ents {
    buf: [0; ENT_BUF],
    start: [0; MAX_ENTITIES],
    len: [0; MAX_ENTITIES],
    kind: [0; MAX_ENTITIES],
    count: 0,
    used: 0,
};

static mut ENT_Q: Ents = EMPTY_ENTS;
static mut ENT_MA: Ents = EMPTY_ENTS;
static mut ENT_GT: Ents = EMPTY_ENTS;

const KIND_URL: u8 = 1;
const KIND_HOST: u8 = 2;
const KIND_IP: u8 = 3;
const KIND_HEX: u8 = 4;

/// Domains that name where evidence came from rather than what was scanned. An
/// answer citing one of these is not answering about a different target.
const SOURCES: [&[u8]; 12] = [
    b"urlhaus", b"abuse", b"virustotal", b"phishtank", b"urlscan", b"openphish", b"safebrowsing",
    b"talosintelligence", b"google", b"cloudflare", b"spamhaus", b"alienvault",
];

#[inline]
fn is_url_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'.' | b':' | b'/' | b'-' | b'_' | b'?' | b'=' | b'&' | b'%' | b'#' | b'@' | b'+' | b'~'
        )
}

fn classify(span: &[u8]) -> Option<u8> {
    if span.len() < 4 {
        return None;
    }
    let mut dots = 0;
    let mut alpha = 0;
    let mut digits = 0;
    let mut hex = 0;
    let mut dashes = 0;
    let mut slashes = 0;
    let mut has_scheme = false;
    let mut i = 0;
    while i < span.len() {
        let b = lower(span[i]);
        match b {
            b'.' => dots += 1,
            b'/' => slashes += 1,
            b'-' => dashes += 1,
            b':' => {
                if i + 2 < span.len() && span[i + 1] == b'/' && span[i + 2] == b'/' {
                    has_scheme = true;
                }
            }
            _ => {}
        }
        if b.is_ascii_alphabetic() {
            alpha += 1;
        }
        if b.is_ascii_digit() {
            digits += 1;
        }
        if b.is_ascii_digit() || (b >= b'a' && b <= b'f') {
            hex += 1;
        }
        i += 1;
    }
    if has_scheme {
        return Some(KIND_URL);
    }
    if dots == 3 && alpha == 0 && digits >= 4 && slashes == 0 {
        return Some(KIND_IP);
    }
    if hex == span.len() && span.len() >= 16 {
        return Some(KIND_HEX);
    }
    if dashes >= 3 && dots == 0 && hex + dashes == span.len() && span.len() >= 20 {
        return Some(KIND_HEX); // uuid
    }
    if dots >= 1 && alpha >= 3 && slashes == 0 {
        // domain-like: at least two alphabetic characters after the final dot
        let mut last_dot = 0;
        let mut j = 0;
        while j < span.len() {
            if span[j] == b'.' {
                last_dot = j;
            }
            j += 1;
        }
        let tld = &span[last_dot + 1..];
        if tld.len() >= 2 && tld.iter().all(|b| b.is_ascii_alphabetic()) {
            return Some(KIND_HOST);
        }
    }
    if dots >= 1 && slashes >= 1 && alpha >= 3 {
        return Some(KIND_URL);
    }
    None
}

fn push_entity(ents: &mut Ents, span: &[u8], kind: u8) {
    // normalise: lowercase, drop scheme, drop www., drop trailing separators
    let mut begin = 0usize;
    let mut end = span.len();
    let mut i = 0;
    while i + 2 < span.len() {
        if span[i] == b':' && span[i + 1] == b'/' && span[i + 2] == b'/' {
            begin = i + 3;
            break;
        }
        i += 1;
    }
    if end > begin + 4 {
        let w = &span[begin..begin + 4];
        if lower(w[0]) == b'w' && lower(w[1]) == b'w' && lower(w[2]) == b'w' && w[3] == b'.' {
            begin += 4;
        }
    }
    while end > begin && matches!(span[end - 1], b'.' | b'/' | b',' | b')' | b';' | b':' | b'?') {
        end -= 1;
    }
    if end <= begin {
        return;
    }
    let len = end - begin;
    if ents.count >= MAX_ENTITIES || ents.used + len > ENT_BUF {
        return;
    }
    let at = ents.used;
    let mut j = 0;
    while j < len {
        ents.buf[at + j] = lower(span[begin + j]);
        j += 1;
    }
    // drop duplicates
    let mut e = 0;
    while e < ents.count {
        let s = ents.start[e] as usize;
        let l = ents.len[e] as usize;
        if l == len {
            let mut same = true;
            let mut k = 0;
            while k < len {
                if ents.buf[s + k] != ents.buf[at + k] {
                    same = false;
                    break;
                }
                k += 1;
            }
            if same {
                return;
            }
        }
        e += 1;
    }
    ents.start[ents.count] = at as u16;
    ents.len[ents.count] = len as u16;
    ents.kind[ents.count] = kind;
    ents.count += 1;
    ents.used = at + len;
}

fn extract_entities(text: &[u8], ents: &mut Ents) {
    ents.count = 0;
    ents.used = 0;
    let mut i = 0;
    while i < text.len() && ents.count < MAX_ENTITIES {
        if !is_url_byte(text[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < text.len() && is_url_byte(text[i]) {
            i += 1;
        }
        let span = &text[start..i];
        if let Some(kind) = classify(span) {
            push_entity(ents, span, kind);
        }
    }
}

fn entity_is_source(ents: &Ents, index: usize) -> bool {
    let s = ents.start[index] as usize;
    let l = ents.len[index] as usize;
    for name in SOURCES.iter() {
        if l >= name.len() {
            let mut off = 0;
            while off + name.len() <= l {
                let mut same = true;
                let mut k = 0;
                while k < name.len() {
                    if ents.buf[s + off + k] != name[k] {
                        same = false;
                        break;
                    }
                    k += 1;
                }
                if same {
                    return true;
                }
                off += 1;
            }
        }
    }
    false
}

fn entity_eq(a: &Ents, ai: usize, b: &Ents, bi: usize) -> bool {
    let (s1, l1) = (a.start[ai] as usize, a.len[ai] as usize);
    let (s2, l2) = (b.start[bi] as usize, b.len[bi] as usize);
    if l1 != l2 {
        return false;
    }
    let mut k = 0;
    while k < l1 {
        if a.buf[s1 + k] != b.buf[s2 + k] {
            return false;
        }
        k += 1;
    }
    true
}

/// Substring search for a normalised entity inside a raw text, case-insensitive.
fn text_contains(text: &[u8], ents: &Ents, index: usize) -> bool {
    let (s, l) = (ents.start[index] as usize, ents.len[index] as usize);
    if l == 0 || l > text.len() {
        return false;
    }
    let mut i = 0;
    while i + l <= text.len() {
        let mut k = 0;
        let mut same = true;
        while k < l {
            if lower(text[i + k]) != ents.buf[s + k] {
                same = false;
                break;
            }
            k += 1;
        }
        if same {
            return true;
        }
        i += 1;
    }
    false
}

/// Two entities are confusable when swapping one for the other is the mistake a
/// miner actually makes: same kind, and either near-identical length (hashes,
/// ids) or heavy character overlap (lookalike domains, sibling paths).
fn entity_confusable(a: &Ents, ai: usize, b: &Ents, bi: usize) -> bool {
    if a.kind[ai] != b.kind[bi] {
        return false;
    }
    let (s1, l1) = (a.start[ai] as usize, a.len[ai] as usize);
    let (s2, l2) = (b.start[bi] as usize, b.len[bi] as usize);
    if l1 < 4 || l2 < 4 {
        return false;
    }
    if a.kind[ai] == KIND_HEX && l1 == l2 {
        return true;
    }
    let longer = if l1 > l2 { l1 } else { l2 };
    let shorter = if l1 > l2 { l2 } else { l1 };
    if (shorter as f32) < 0.6 * longer as f32 {
        return false;
    }
    let mut shared = 0u32;
    let mut total = 0u32;
    let mut i = 0;
    while i + 3 <= l1 {
        total += 1;
        let mut j = 0;
        while j + 3 <= l2 {
            if a.buf[s1 + i] == b.buf[s2 + j]
                && a.buf[s1 + i + 1] == b.buf[s2 + j + 1]
                && a.buf[s1 + i + 2] == b.buf[s2 + j + 2]
            {
                shared += 1;
                break;
            }
            j += 1;
        }
        i += 1;
    }
    total > 0 && (shared as f32 / total as f32) >= 0.45
}

// ---------------------------------------------------------------- polarity

const MAL: [&[u8]; 22] = [
    b"malicious", b"malware", b"phishing", b"phish", b"ransomware", b"trojan", b"spyware",
    b"scam", b"fraudulent", b"unsafe", b"dangerous", b"harmful", b"blacklisted", b"compromised",
    b"skimmer", b"spoofed", b"spoof", b"exploit", b"botnet", b"c2", b"malvertising", b"phished",
];
const SUS: [&[u8]; 10] = [
    b"suspicious", b"questionable", b"risky", b"caution", b"cautious", b"untrusted",
    b"inconclusive", b"borderline", b"unverified", b"newly",
];
const CLEAN: [&[u8]; 14] = [
    b"safe", b"benign", b"clean", b"harmless", b"legitimate", b"legit", b"trusted", b"trustworthy",
    b"reputable", b"genuine", b"innocuous", b"nonmalicious", b"whitelisted", b"unflagged",
];
const UNKNOWN: [&[u8]; 17] = [
    b"unknown", b"unavailable", b"timeout", b"timed", b"unresolved", b"undetermined",
    b"processing", b"pending", b"inaccessible", b"nxdomain", b"unreachable", b"indeterminate",
    b"failed", b"error", b"errored", b"unable", b"inconclusive",
];
/// The second axis a URL_SCAN answer turns on: whether the record exists, not
/// whether the target is dangerous. "in PhishTank but not yet verified" and
/// "in PhishTank, verified" agree on the verdict and differ on the decision.
const CONFIRM: [&[u8]; 23] = [
    b"verified", b"verification", b"confirmed", b"confirms", b"listed", b"lists", b"present",
    b"resolved", b"complete", b"completed", b"finished", b"corroborated", b"record", b"records",
    b"available", b"verifies", b"detected", b"detects", b"match", b"matches", b"matching",
    b"identical", b"equal",
];
/// Which way a quantity moved. A miner that says "rise" for a ground truth of
/// "fall" has reused every other word in the sentence.
const DIR_UP: [&[u8]; 31] = [
    b"rise", b"rises", b"rising", b"rose", b"risen", b"increase", b"increases", b"increasing",
    b"increased", b"higher", b"gain", b"gains", b"grew", b"growth", b"surge", b"surges",
    b"bullish", b"climb", b"climbs", b"appreciate", b"strengthened", b"strengthen", b"strengthens",
    b"appreciated", b"rallied", b"rally", b"firmer", b"stronger", b"up", b"upward", b"grow",
];
const DIR_DOWN: [&[u8]; 31] = [
    b"fall", b"falls", b"falling", b"fell", b"fallen", b"decrease", b"decreases", b"decreasing",
    b"decreased", b"lower", b"drop", b"drops", b"decline", b"declines", b"declining", b"shrink",
    b"plunge", b"bearish", b"dip", b"depreciate", b"weakened", b"weaken", b"weakens",
    b"depreciated", b"softened", b"slid", b"weaker", b"softer", b"down", b"downward", b"sank",
];
/// Whether the claim holds at all, and whether the thing is what it claims to
/// be. One axis: both are read off the same yes/no words.
/// Does the claim hold: the yes/no the question actually asked for.
const AFFIRM: [&[u8]; 15] = [
    b"yes", b"correct", b"true", b"supported", b"supports", b"support", b"accurate",
    b"upheld", b"substantiated", b"affirmative", b"right", b"expected", b"likely", b"probable",
    b"anticipated",
];
const DENIAL: [&[u8]; 14] = [
    b"no", b"incorrect", b"false", b"refuted", b"refutes", b"debunked", b"denied", b"denies",
    b"unsupported", b"wrong", b"misleading", b"unlikely", b"improbable", b"contradicted",
];
/// Is the thing what it claims to be. Read separately from the yes/no, because
/// "No, written by a human" is negative on one and positive on the other.
const REAL: [&[u8]; 16] = [
    b"genuine", b"authentic", b"real", b"human", b"legitimate", b"original", b"unmodified",
    b"untampered", b"valid", b"unaltered", b"organic", b"handwritten", b"unedited", b"uncut",
    b"raw", b"pristine",
];
const FAKE: [&[u8]; 19] = [
    b"fake", b"forged", b"forgery", b"synthetic", b"synthesis", b"synthesised", b"fabricated",
    b"manipulated", b"manipulation", b"deepfake", b"doctored", b"tampered", b"tampering",
    b"counterfeit", b"invalid", b"machine", b"ai", b"altered", b"generated",
];

const DENY: [&[u8]; 11] = [
    b"unlisted", b"delisted", b"absent", b"missing", b"removed", b"expired", b"differ",
    b"differs", b"different", b"mismatch", b"mismatched",
];
/// Scale words. `3.1 trillion` and `$3.1T` are the same number; `3.1 billion`
/// is a different one by a factor of a thousand.
const MAG_3: [&[u8]; 3] = [b"thousand", b"k", b"thousands"];
const MAG_6: [&[u8]; 4] = [b"million", b"m", b"mm", b"millions"];
const MAG_9: [&[u8]; 4] = [b"billion", b"b", b"bn", b"billions"];
const MAG_12: [&[u8]; 3] = [b"trillion", b"t", b"tn"];

fn magnitude_of(text: &[u8], toks: &Toks, t: usize) -> u8 {
    let (s, l) = (toks.start[t] as usize, toks.len[t] as usize);
    if matches_list(text, s, l, &MAG_3) {
        3
    } else if matches_list(text, s, l, &MAG_6) {
        6
    } else if matches_list(text, s, l, &MAG_9) {
        9
    } else if matches_list(text, s, l, &MAG_12) {
        12
    } else {
        0
    }
}

/// A figure that carries the wrong scale is wrong by a factor of a thousand or
/// more, however well the sentence around it matches.
fn magnitude_conflict(gt_text: &[u8], gt: &Toks, ma_text: &[u8], ma: &Toks) -> bool {
    let mut t = 0usize;
    while t < gt.count {
        if gt.numeric[t] && t + 1 < gt.count {
            let gt_mag = magnitude_of(gt_text, gt, t + 1);
            if gt_mag > 0 {
                let mut u = 0usize;
                while u < ma.count {
                    if words_match(gt_text, gt, t, ma_text, ma, u) && u + 1 < ma.count {
                        let ma_mag = magnitude_of(ma_text, ma, u + 1);
                        if ma_mag > 0 && ma_mag != gt_mag {
                            return true;
                        }
                    }
                    u += 1;
                }
            }
        }
        t += 1;
    }
    false
}

const NEGATORS: [&[u8]; 16] = [
    b"not", b"no", b"never", b"none", b"nothing", b"neither", b"nor", b"without", b"isnt",
    b"wasnt", b"doesnt", b"dont", b"cannot", b"cant", b"rather", b"instead",
];
const POST_NEG: [&[u8]; 4] = [b"false", b"absent", b"unlisted", b"none"];
const BOUNDARY: [&[u8]; 6] = [b"but", b"however", b"though", b"although", b"while", b"whereas"];

/// A verdict word is negated by a negator up to four tokens back (stopped at a
/// clause boundary), by a leading zero count, or by a `false` immediately after,
/// which is how every JSON-shaped miner in this intent says no.
fn negated(text: &[u8], toks: &Toks, t: usize) -> bool {
    let mut back = 1usize;
    let mut seen_figure = false;
    while back <= 4 && back <= t {
        let p = t - back;
        let ps = toks.start[p] as usize;
        let pl = toks.len[p] as usize;
        if matches_list(text, ps, pl, &BOUNDARY) || toks.bnd[p] {
            break;
        }
        // the nearest count in front of a verdict word owns it: "0 malicious" and
        // "0 calling it harmless" are denials, "72 engines report harmless" is not
        if toks.numeric[p] && !seen_figure {
            seen_figure = true;
            if pl == 1 && text[ps] == b'0' {
                return true;
            }
        }
        if matches_list(text, ps, pl, &NEGATORS) {
            return true;
        }
        back += 1;
    }
    let mut fwd = 1usize;
    while fwd <= 2 && t + fwd < toks.count {
        let n = t + fwd;
        let ns = toks.start[n] as usize;
        let nl = toks.len[n] as usize;
        if matches_list(text, ns, nl, &POST_NEG) {
            return true;
        }
        fwd += 1;
    }
    false
}

struct Polarity {
    value: f32,
    strength: f32,
    unknown: f32,
}

/// Reads the verdict axis. `value` runs from -1 (clean) to +1 (malicious) with
/// suspicious in between; `unknown` is the separate "no verdict was produced"
/// axis, which contradicts every definite verdict including a clean one.
fn polarity(text: &[u8], toks: &Toks) -> Polarity {
    let mut sum = 0.0f32;
    let mut votes = 0.0f32;
    let mut unknown = 0.0f32;
    let mut definite = 0.0f32;
    let mut t = 0usize;
    while t < toks.count {
        let start = toks.start[t] as usize;
        let len = toks.len[t] as usize;
        let axis = if matches_list_infl(text, start, len, &MAL) {
            1
        } else if matches_list_infl(text, start, len, &CLEAN) {
            2
        } else if matches_list_infl(text, start, len, &SUS) {
            3
        } else if matches_list_infl(text, start, len, &UNKNOWN) {
            4
        } else {
            0
        };
        if axis != 0 {
            let neg = negated(text, toks, t);
            match axis {
                1 => {
                    sum += if neg { -1.0 } else { 1.0 };
                    votes += 1.0;
                    definite += 1.0;
                }
                2 => {
                    sum += if neg { 1.0 } else { -1.0 };
                    votes += 1.0;
                    definite += 1.0;
                }
                3 => {
                    if !neg {
                        sum += 0.3;
                        votes += 1.0;
                        definite += 0.5;
                    }
                }
                _ => {
                    if !neg {
                        unknown += 1.0;
                    }
                }
            }
        }
        t += 1;
    }
    let value = if votes > 0.0 { sum / votes } else { 0.0 };
    Polarity {
        value,
        strength: if votes > 0.0 { 1.0 } else { 0.0 },
        unknown: if unknown > 0.0 && definite < 1.0 {
            1.0
        } else if unknown > 0.0 {
            0.5
        } else {
            0.0
        },
    }
}

/// A two-sided axis: words that vote one way, words that vote the other, each
/// flipped by a negation in front of it. Reported with its vote count so an
/// axis nobody spoke on is never treated as a contradiction.
fn axis(text: &[u8], toks: &Toks, plus: &[&[u8]], minus: &[&[u8]]) -> (f32, f32) {
    axis_full(text, toks, plus, minus).0
}

/// Returns the reading and whether the votes cancelled out. Two words pulling
/// opposite ways is a compound claim ("the image is real, the caption is not");
/// three or more spanning every option is an answer asserting everything at once.
fn axis_full(
    text: &[u8],
    toks: &Toks,
    plus: &[&[u8]],
    minus: &[&[u8]],
) -> ((f32, f32), bool) {
    let (value, votes, cancelled) = axis_inner(text, toks, plus, minus);
    ((value, votes), cancelled)
}

fn axis_inner(text: &[u8], toks: &Toks, plus: &[&[u8]], minus: &[&[u8]]) -> (f32, f32, bool) {
    let mut sum = 0.0f32;
    let mut votes = 0.0f32;
    let mut t = 0usize;
    while t < toks.count {
        let start = toks.start[t] as usize;
        let len = toks.len[t] as usize;
        let side = if matches_list_infl(text, start, len, plus) {
            1.0
        } else if matches_list_infl(text, start, len, minus) {
            -1.0
        } else {
            0.0
        };
        let bare = matches_list(text, start, len, &[b"no", b"yes"]);
        if side != 0.0 && (!bare || toks.bnd[t] || t == 0) {
            // "No, the image is authentic" answers the question; "no sign of
            // manipulation" is a negation inside a clause and votes on nothing
            sum += if negated(text, toks, t) { -side } else { side };
            votes += 1.0;
        }
        t += 1;
    }
    let value = if votes > 0.0 { sum / votes } else { 0.0 };
    let mut spread = value;
    if spread < 0.0 {
        spread = -spread;
    }
    let cancelled = votes >= 2.0 && votes < 3.0 && spread < 0.2;
    (value, votes, cancelled)
}

fn axis_gap(a: (f32, f32), b: (f32, f32)) -> f32 {
    if a.1 <= 0.0 || b.1 <= 0.0 {
        return 0.0;
    }
    let d = a.0 - b.0;
    if d < 0.0 {
        -d
    } else {
        d
    }
}

// ----------------------------------------------------------------- numbers

/// Fraction of the ground truth's figures the answer contradicts. A figure that
/// is simply absent is not a contradiction; a figure replaced by a different one
/// is, because that is the difference between blocking a download and allowing it.
fn numeric_conflict(gt_text: &[u8], gt: &Toks, ma_text: &[u8], ma: &Toks) -> f32 {
    let mut expected = 0u32;
    let mut missing = 0u32;
    let mut extra_present = false;
    let mut t = 0usize;
    while t < gt.count {
        if gt.numeric[t] {
            expected += 1;
            let mut found = false;
            let mut u = 0usize;
            while u < ma.count {
                if words_match(gt_text, gt, t, ma_text, ma, u) {
                    found = true;
                    break;
                }
                u += 1;
            }
            if !found {
                missing += 1;
            }
        }
        t += 1;
    }
    if expected == 0 {
        return 0.0;
    }
    let mut u = 0usize;
    while u < ma.count {
        if ma.numeric[u] {
            let mut found = false;
            let mut t2 = 0usize;
            while t2 < gt.count {
                if words_match(ma_text, ma, u, gt_text, gt, t2) {
                    found = true;
                    break;
                }
                t2 += 1;
            }
            if !found {
                extra_present = true;
                break;
            }
        }
        u += 1;
    }
    if missing == 0 || !extra_present {
        return 0.0;
    }
    missing as f32 / expected as f32
}

/// The label a figure is attached to. A miner that reports the right numbers
/// against the wrong fields ("26 malicious, 41 harmless" for "41 malicious, 26
/// harmless") passes every set comparison and inverts the decision, so each
/// figure is checked against the content word it sits next to.
fn label_of(text: &[u8], toks: &Toks, t: usize) -> Option<usize> {
    label_within(text, toks, t, 3).map(|(i, _)| i)
}

/// The field a figure is attached to, and whether it sits behind it.
fn label_within(text: &[u8], toks: &Toks, t: usize, reach: usize) -> Option<(usize, bool)> {
    let usable = |i: usize| {
        !toks.numeric[i] && toks.weight[i] >= 1.0 && magnitude_of(text, toks, i) == 0
    };
    // behind first: "Arbitrum, at 2.6 billion against Base's 1.9 billion" binds
    // each figure to the name in front of it, not to the scale word after it
    let mut back = 1usize;
    while back <= reach && back <= t {
        let p = t - back;
        if usable(p) {
            return Some((p, true));
        }
        back += 1;
    }
    let mut fwd = 1usize;
    while fwd <= reach && t + fwd < toks.count {
        let n = t + fwd;
        if usable(n) {
            return Some((n, false));
        }
        fwd += 1;
    }
    None
}

fn numeric_slot_conflict(gt_text: &[u8], gt: &Toks, ma_text: &[u8], ma: &Toks) -> f32 {
    let mut slots = 0u32;
    let mut swapped = 0u32;
    let mut t = 0usize;
    while t < gt.count {
        if !gt.numeric[t] {
            t += 1;
            continue;
        }
        let (gt_label, gt_behind) = match label_within(gt_text, gt, t, 3) {
            Some(l) => l,
            None => {
                t += 1;
                continue;
            }
        };
        slots += 1;
        let mut same_value_wrong_label: Option<usize> = None;
        let mut settled = false;
        let mut u = 0usize;
        while u < ma.count {
            if words_match(gt_text, gt, t, ma_text, ma, u) {
                match label_within(ma_text, ma, u, 2) {
                    Some((ma_label, ma_behind)) => {
                        if words_match(gt_text, gt, gt_label, ma_text, ma, ma_label) {
                            settled = true; // right figure against the right field
                            break;
                        }
                        if same_value_wrong_label.is_none() && ma_behind == gt_behind {
                            let mut serves = 0u32;
                            let mut m = 0usize;
                            while m < ma.count {
                                if ma.numeric[m] {
                                    if let Some((other, _)) = label_within(ma_text, ma, m, 2) {
                                        if other == ma_label {
                                            serves += 1;
                                        }
                                    }
                                }
                                m += 1;
                            }
                            if serves <= 1 {
                                same_value_wrong_label = Some(ma_label);
                            }
                        }
                    }
                    None => {
                        settled = true;
                        break;
                    }
                }
            }
            u += 1;
        }
        if !settled {
            if let Some(ma_label) = same_value_wrong_label {
                // a swap only counts when the answer hung this figure on a field the
                // ground truth used for a different one
                let mut k = 0usize;
                while k < gt.count {
                    if gt.numeric[k] && k != t {
                        if let Some(other) = label_of(gt_text, gt, k) {
                            if words_match(gt_text, gt, other, ma_text, ma, ma_label) {
                                swapped += 1;
                                break;
                            }
                        }
                    }
                    k += 1;
                }
            }
        }
        t += 1;
    }
    if slots == 0 {
        0.0
    } else {
        swapped as f32 / slots as f32
    }
}

// ------------------------------------------------------------- similarity

struct Overlap {
    recall: f32,
    precision: f32,
    bigram: f32,
    pairs: u32,
    /// Recall counted only over the part of the ground truth the question did
    /// not already contain, and how much of the ground truth that part is. An
    /// answer that repeats the question scores well on the whole sentence and
    /// nothing at all on this.
    novel: f32,
    novel_share: f32,
}

// ---------------------------------------------------------------- meaning

/// GloVe rows, L2-normalised to int8 and keyed by the same FNV-1a hash the
/// tokeniser computes. Built by tools/pack_vectors.py.
static VECTORS: &[u8] = include_bytes!("vectors.bin");
const VEC_DIM: usize = 300;
/// Below this cosine two words are merely on the same topic.
const VEC_NEAR: f32 = 0.45;
/// What a near neighbour is worth against the word the ground truth actually
/// used. Never all of it.
const VEC_CREDIT: f32 = 0.75;
/// Bound on the pass, so a 76 KB answer costs what a short one costs.
const VEC_SCAN: usize = 96;

fn vector_count() -> usize {
    u32::from_le_bytes([VECTORS[4], VECTORS[5], VECTORS[6], VECTORS[7]]) as usize
}

fn vector_row(hash: u32) -> Option<usize> {
    if VECTORS.len() < 12 || VECTORS[0] != b'V' {
        return None;
    }
    let count = vector_count();
    let (mut lo, mut hi) = (0usize, count);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let at = 12 + mid * 4;
        let key = u32::from_le_bytes([
            VECTORS[at],
            VECTORS[at + 1],
            VECTORS[at + 2],
            VECTORS[at + 3],
        ]);
        if key == hash {
            return Some(mid);
        }
        if key < hash {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    None
}

/// Both rows are unit vectors quantised to int8, so the dot product over 127
/// squared is the cosine.
fn vector_cosine(a: usize, b: usize) -> f32 {
    let base = 12 + vector_count() * 4;
    let (pa, pb) = (base + a * VEC_DIM, base + b * VEC_DIM);
    let mut dot = 0i32;
    let mut i = 0usize;
    while i < VEC_DIM {
        dot += (VECTORS[pa + i] as i8 as i32) * (VECTORS[pb + i] as i8 as i32);
        i += 1;
    }
    dot as f32 / (127.0 * 127.0)
}

/// Two words on opposite sides of any axis are never neighbours, however close
/// their vectors are. GloVe puts `increase` and `decrease` at 0.81, closer than
/// `rise` and `increase` at 0.67, because it reads topic and not direction. The
/// axes are what this module has instead.
fn axis_opposed(a_text: &[u8], a: &Toks, ai: usize, b_text: &[u8], b: &Toks, bi: usize) -> bool {
    let (sa, la) = (a.start[ai] as usize, a.len[ai] as usize);
    let (sb, lb) = (b.start[bi] as usize, b.len[bi] as usize);
    let pairs: [(&[&[u8]], &[&[u8]]); 6] = [
        (&MAL, &CLEAN),
        (&CONFIRM, &DENY),
        (&DIR_UP, &DIR_DOWN),
        (&AFFIRM, &DENIAL),
        (&REAL, &FAKE),
        (&SUS, &CLEAN),
    ];
    for (plus, minus) in pairs.iter() {
        let a_plus = matches_list_infl(a_text, sa, la, plus);
        let a_minus = matches_list_infl(a_text, sa, la, minus);
        let b_plus = matches_list_infl(b_text, sb, lb, plus);
        let b_minus = matches_list_infl(b_text, sb, lb, minus);
        if (a_plus && b_minus) || (a_minus && b_plus) {
            return true;
        }
    }
    false
}

/// How much of a lexical hit the nearest word in `b` is worth for token `ai`.
fn nearest_meaning(a_text: &[u8], a: &Toks, ai: usize, b_text: &[u8], b: &Toks) -> f32 {
    if a.numeric[ai] || a.weight[ai] < 1.0 {
        return 0.0;
    }
    let own = match vector_row(a.hash[ai]) {
        Some(r) => r,
        None => return 0.0,
    };
    let mut best = 0.0f32;
    let mut j = 0usize;
    let mut scanned = 0usize;
    while j < b.count && scanned < VEC_SCAN {
        if !b.numeric[j] && b.weight[j] >= 1.0 {
            scanned += 1;
            if let Some(other) = vector_row(b.hash[j]) {
                let cos = vector_cosine(own, other);
                if cos > best && !axis_opposed(a_text, a, ai, b_text, b, j) {
                    best = cos;
                }
            }
        }
        j += 1;
    }
    if best <= VEC_NEAR {
        return 0.0;
    }
    VEC_CREDIT * clamp01((best - VEC_NEAR) / (1.0 - VEC_NEAR))
}

fn found_in(a_text: &[u8], a: &Toks, ai: usize, b_text: &[u8], b: &Toks) -> bool {
    let mut i = 0usize;
    while i < b.count {
        if words_match(a_text, a, ai, b_text, b, i) {
            return true;
        }
        i += 1;
    }
    false
}

/// Index of the next content token at or after `from`, skipping filler.
fn next_content(toks: &Toks, from: usize) -> Option<usize> {
    let mut i = from;
    while i < toks.count {
        if toks.weight[i] >= 1.0 {
            return Some(i);
        }
        i += 1;
    }
    None
}

static mut ACRO_GT: [bool; MAX_TOKENS] = [false; MAX_TOKENS];
static mut ACRO_MA: [bool; MAX_TOKENS] = [false; MAX_TOKENS];

fn is_acronym(text: &[u8], toks: &Toks, i: usize) -> bool {
    let (s, l) = (toks.start[i] as usize, toks.len[i] as usize);
    let unit_letter = l == 1 && i > 0 && toks.numeric[i - 1];
    if !((2..=5).contains(&l) || unit_letter) || toks.numeric[i] {
        return false;
    }
    let mut k = 0;
    while k < l {
        if !text[s + k].is_ascii_uppercase() {
            return false;
        }
        k += 1;
    }
    true
}

/// `US` for `United States`, `EMH` for `efficient market hypothesis`: an answer
/// that abbreviates the ground truth has still named it. Marks the tokens on
/// both sides that an abbreviation on the other side accounts for.
fn bridge_acronyms(
    a_text: &[u8],
    a: &Toks,
    a_hit: &mut [bool; MAX_TOKENS],
    b_text: &[u8],
    b: &Toks,
    b_hit: &mut [bool; MAX_TOKENS],
) {
    let mut i = 0usize;
    while i < a.count {
        if is_acronym(a_text, a, i) {
            let (s, l) = (a.start[i] as usize, a.len[i] as usize);
            let mut start = 0usize;
            while start + l <= b.count {
                let mut k = 0usize;
                let mut all = true;
                while k < l {
                    let t = start + k;
                    let ts = b.start[t] as usize;
                    if b.weight[t] < 1.0 || b.numeric[t] || lower(b_text[ts]) != lower(a_text[s + k])
                    {
                        all = false;
                        break;
                    }
                    k += 1;
                }
                if all {
                    a_hit[i] = true;
                    let mut k2 = 0usize;
                    while k2 < l {
                        b_hit[start + k2] = true;
                        k2 += 1;
                    }
                    break;
                }
                start += 1;
            }
            if !a_hit[i] && l <= 3 {
                // a country or currency code standing in for one longer word
                let mut t = 0usize;
                while t < b.count {
                    let (ts, tl) = (b.start[t] as usize, b.len[t] as usize);
                    if b.weight[t] >= 1.0 && tl >= 5 {
                        let mut k = 0usize;
                        let mut same = true;
                        while k < l {
                            if lower(b_text[ts + k]) != lower(a_text[s + k]) {
                                same = false;
                                break;
                            }
                            k += 1;
                        }
                        if same {
                            a_hit[i] = true;
                            b_hit[t] = true;
                            break;
                        }
                    }
                    t += 1;
                }
            }
        }
        i += 1;
    }
}

/// A capitalised name that appears in neither the question nor the ground truth
/// is an identity the ground truth does not support: "a Cloudflare edge address"
/// where the truth says "a Tor exit node". Sentence-openers and abbreviations are
/// excluded, the first is already handled by the acronym bridge.
fn foreign_names(
    ma_text: &[u8],
    ma: &Toks,
    gt_text: &[u8],
    gt: &Toks,
    q_text: &[u8],
    q: &Toks,
) -> u32 {
    let mut count = 0u32;
    let mut i = 0usize;
    while i < ma.count {
        let (s, l) = (ma.start[i] as usize, ma.len[i] as usize);
        let opens_sentence = i == 0 || ma.bnd[i - 1];
        if l >= 4
            && !opens_sentence
            && !ma.numeric[i]
            && ma_text[s].is_ascii_uppercase()
            && !ma_text[s + 1].is_ascii_uppercase()
            && !found_in(ma_text, ma, i, gt_text, gt)
            && !found_in(ma_text, ma, i, q_text, q)
        {
            count += 1;
        }
        i += 1;
    }
    count
}

fn weighted_overlap(
    gt_text: &[u8],
    gt: &Toks,
    ma_text: &[u8],
    ma: &Toks,
    q_text: &[u8],
    q: &Toks,
) -> Overlap {
    let (acro_gt, acro_ma) = unsafe {
        (
            &mut *core::ptr::addr_of_mut!(ACRO_GT),
            &mut *core::ptr::addr_of_mut!(ACRO_MA),
        )
    };
    for slot in acro_gt.iter_mut() {
        *slot = false;
    }
    for slot in acro_ma.iter_mut() {
        *slot = false;
    }
    bridge_acronyms(ma_text, ma, acro_ma, gt_text, gt, acro_gt);
    bridge_acronyms(gt_text, gt, acro_gt, ma_text, ma, acro_ma);

    let mut recall_total = 0.0f32;
    let mut recall_hit = 0.0f32;
    let novel_total = 0.0f32;
    let novel_hit = 0.0f32;
    let mut t = 0usize;
    while t < gt.count {
        // the part of the ground truth the question already gave away is prompt,
        // not answer, so it is discounted rather than counted as recall
        let in_question = found_in(gt_text, gt, t, q_text, q);
        let weight = if in_question {
            gt.weight[t] * gt.weight[t] * 0.15
        } else {
            gt.weight[t] * gt.weight[t]
        };
        recall_total += weight;
        if acro_gt[t] || found_in(gt_text, gt, t, ma_text, ma) {
            recall_hit += weight;
        }
        t += 1;
    }

    let mut precision_total = 0.0f32;
    let mut precision_hit = 0.0f32;
    let mut u = 0usize;
    while u < ma.count {
        precision_total += ma.weight[u] * ma.weight[u];
        let mut credit = if acro_ma[u]
            || found_in(ma_text, ma, u, gt_text, gt)
            || found_in(ma_text, ma, u, q_text, q)
        {
            1.0
        } else {
            nearest_meaning(ma_text, ma, u, gt_text, gt)
        };
        if credit > 1.0 {
            credit = 1.0;
        }
        precision_hit += ma.weight[u] * ma.weight[u] * credit;
        u += 1;
    }

    // adjacency: an answer carrying every content word of the ground truth and
    // none of its pairings has rearranged the claim rather than restated it
    let mut pairs = 0u32;
    let mut pairs_hit = 0u32;
    let mut i = 0usize;
    while let Some(a) = next_content(gt, i) {
        let b = match next_content(gt, a + 1) {
            Some(b) => b,
            None => break,
        };
        pairs += 1;
        let mut j = 0usize;
        while let Some(c) = next_content(ma, j) {
            let d = match next_content(ma, c + 1) {
                Some(d) => d,
                None => break,
            };
            if words_match(gt_text, gt, a, ma_text, ma, c) && words_match(gt_text, gt, b, ma_text, ma, d)
            {
                pairs_hit += 1;
                break;
            }
            j = c + 1;
        }
        i = a + 1;
    }

    Overlap {
        recall: if recall_total > 0.0 {
            recall_hit / recall_total
        } else {
            0.0
        },
        precision: if precision_total > 0.0 {
            precision_hit / precision_total
        } else {
            0.0
        },
        bigram: if pairs > 0 {
            pairs_hit as f32 / pairs as f32
        } else {
            1.0
        },
        pairs,
        novel: if novel_total > 0.0 {
            novel_hit / novel_total
        } else {
            1.0
        },
        novel_share: if recall_total > 0.0 {
            novel_total / recall_total
        } else {
            0.0
        },
    }
}

#[inline]
fn clamp01(x: f32) -> f32 {
    if x < 0.0 {
        0.0
    } else if x > 1.0 {
        1.0
    } else {
        x
    }
}

#[inline]
fn smoothstep(x: f32) -> f32 {
    let x = if x < 0.0 {
        0.0
    } else if x > 1.0 {
        1.0
    } else {
        x
    };
    x * x * (3.0 - 2.0 * x)
}

fn word_bytes_equal(a_text: &[u8], a: &Toks, b_text: &[u8], b: &Toks) -> bool {
    if a.count != b.count {
        return false;
    }
    let mut i = 0;
    while i < a.count {
        if !eq_ci_across(
            a_text,
            (a.start[i] as usize, a.len[i] as usize),
            b_text,
            (b.start[i] as usize, b.len[i] as usize),
        ) {
            return false;
        }
        i += 1;
    }
    true
}

// --------------------------------------------------------------- scoring

/// The parts behind a score. Most fields exist for the `probe` build.
#[allow(dead_code)]
struct Eval {
    score: f32,
    base: f32,
    recall: f32,
    precision: f32,
    trigram: f32,
    penalty: f32,
    verdict_gap: f32,
    confirm_gap: f32,
    direction_gap: f32,
    affirm_gap: f32,
    axis_support: f32,
    bigram: f32,
    real_gap: f32,
    conflict: f32,
    slot_conflict: f32,
    target_conflict: f32,
    unknown_gap: f32,
}

const ZERO_EVAL: Eval = Eval {
    score: 0.0,
    base: 0.0,
    recall: 0.0,
    precision: 0.0,
    trigram: 0.0,
    penalty: 1.0,
    verdict_gap: 0.0,
    confirm_gap: 0.0,
    direction_gap: 0.0,
    affirm_gap: 0.0,
    axis_support: 0.0,
    bigram: 0.0,
    real_gap: 0.0,
    conflict: 0.0,
    slot_conflict: 0.0,
    target_conflict: 0.0,
    unknown_gap: 0.0,
};

fn score(question: &[u8], ground_truth: &[u8], answer: &[u8]) -> f32 {
    evaluate(question, ground_truth, answer).score
}

fn evaluate(question: &[u8], ground_truth: &[u8], answer: &[u8]) -> Eval {
    let (q, gt, ma) = unsafe {
        (
            &mut *core::ptr::addr_of_mut!(TOK_Q),
            &mut *core::ptr::addr_of_mut!(TOK_GT),
            &mut *core::ptr::addr_of_mut!(TOK_MA),
        )
    };
    tokenize(question, q);
    tokenize(ground_truth, gt);
    tokenize(answer, ma);

    if ma.count == 0 {
        return ZERO_EVAL;
    }
    if word_bytes_equal(ground_truth, gt, answer, ma) {
        return Eval {
            score: 1.0,
            base: 1.0,
            recall: 1.0,
            precision: 1.0,
            trigram: 1.0,
            ..ZERO_EVAL
        };
    }
    if gt.count == 0 {
        return ZERO_EVAL;
    }

    let (eq, ema, egt) = unsafe {
        (
            &mut *core::ptr::addr_of_mut!(ENT_Q),
            &mut *core::ptr::addr_of_mut!(ENT_MA),
            &mut *core::ptr::addr_of_mut!(ENT_GT),
        )
    };
    extract_entities(question, eq);
    extract_entities(answer, ema);
    extract_entities(ground_truth, egt);

    // ---- target binding
    let mut target: Option<(&Ents, usize)> = None;
    let mut i = 0;
    while i < eq.count {
        if !entity_is_source(eq, i) {
            match target {
                Some((_, prev)) if eq.len[prev] >= eq.len[i] => {}
                _ => target = Some((&*eq, i)),
            }
        }
        i += 1;
    }
    if target.is_none() {
        let mut j = 0;
        while j < egt.count {
            if !entity_is_source(egt, j) {
                match target {
                    Some((_, prev)) if egt.len[prev] >= egt.len[j] => {}
                    _ => target = Some((&*egt, j)),
                }
            }
            j += 1;
        }
    }

    let mut target_conflict = false;
    if let Some((owner, index)) = target {
        let bound = text_contains(answer, owner, index);
        if !bound {
            let mut k = 0;
            while k < ema.count {
                if !entity_is_source(ema, k)
                    && !entity_eq(owner, index, ema, k)
                    && entity_confusable(owner, index, ema, k)
                {
                    target_conflict = true;
                    break;
                }
                k += 1;
            }
        }
    }

    // ---- verdict axis
    let p_gt = polarity(ground_truth, gt);
    let p_ma = polarity(answer, ma);
    let mut verdict_gap = 0.0f32;
    if p_gt.strength > 0.0 && p_ma.strength > 0.0 {
        verdict_gap = p_gt.value - p_ma.value;
        if verdict_gap < 0.0 {
            verdict_gap = -verdict_gap;
        }
    }
    let unknown_gap = (p_gt.unknown >= 1.0 && p_ma.unknown <= 0.0 && p_ma.strength > 0.0)
        || (p_ma.unknown >= 1.0 && p_gt.unknown <= 0.0 && p_gt.strength > 0.0);

    // ---- the other axes an answer can flip while reusing every word
    let confirm = axis_gap(
        axis(ground_truth, gt, &CONFIRM, &DENY),
        axis(answer, ma, &CONFIRM, &DENY),
    );
    let confirm_gap = if p_ma.unknown <= 0.0 && p_gt.unknown <= 0.0 {
        confirm
    } else {
        0.0
    };
    let (dir_gt, dir_gt_mix) = axis_full(ground_truth, gt, &DIR_UP, &DIR_DOWN);
    let (dir_ma, dir_ma_mix) = axis_full(answer, ma, &DIR_UP, &DIR_DOWN);
    let direction_gap = if dir_gt_mix || dir_ma_mix {
        0.0
    } else {
        axis_gap(dir_gt, dir_ma)
    };
    let (aff_gt, aff_gt_mix) = axis_full(ground_truth, gt, &AFFIRM, &DENIAL);
    let (aff_ma, aff_ma_mix) = axis_full(answer, ma, &AFFIRM, &DENIAL);
    let affirm_gap = if aff_gt_mix || aff_ma_mix {
        0.0
    } else {
        axis_gap(aff_gt, aff_ma)
    };
    let (real_gt, real_gt_mix) = axis_full(ground_truth, gt, &REAL, &FAKE);
    let (real_ma, real_ma_mix) = axis_full(answer, ma, &REAL, &FAKE);
    let real_gap = if real_gt_mix || real_ma_mix {
        0.0
    } else {
        axis_gap(real_gt, real_ma)
    };

    // how many axes both sides spoke on, and how many they agree on: an answer
    // that asserts the same facts in different words has still answered
    let mut axes_shared = 0u32;
    let mut axes_agreed = 0u32;
    {
        let mut consider = |gap: f32, a: f32, b: f32| {
            if a > 0.0 && b > 0.0 {
                axes_shared += 1;
                if gap < 0.2 {
                    axes_agreed += 1;
                }
            }
        };
        consider(verdict_gap, p_gt.strength, p_ma.strength);
        consider(confirm_gap, axis(ground_truth, gt, &CONFIRM, &DENY).1, axis(answer, ma, &CONFIRM, &DENY).1);
        consider(direction_gap, dir_gt.1, dir_ma.1);
        consider(affirm_gap, aff_gt.1, aff_ma.1);
        consider(real_gap, real_gt.1, real_ma.1);
    }

    // ---- figures
    let conflict = numeric_conflict(ground_truth, gt, answer, ma);
    let slot_conflict = numeric_slot_conflict(ground_truth, gt, answer, ma);
    let scale_conflict = magnitude_conflict(ground_truth, gt, answer, ma);

    // ---- textual base
    let overlap = weighted_overlap(ground_truth, gt, answer, ma, question, q);
    let (bits_gt, bits_ma) = unsafe {
        (
            &mut *core::ptr::addr_of_mut!(TRI_GT),
            &mut *core::ptr::addr_of_mut!(TRI_MA),
        )
    };
    let n_gt = trigram_bits(ground_truth, gt, bits_gt);
    let n_ma = trigram_bits(answer, ma, bits_ma);
    let shared = popcount_and(bits_gt, bits_ma);
    let dice = if n_gt + n_ma > 0 {
        2.0 * shared as f32 / (n_gt + n_ma) as f32
    } else {
        0.0
    };
    let coverage = if n_gt > 0 {
        shared as f32 / n_gt as f32
    } else {
        0.0
    };
    let trigram = if dice > coverage { dice } else { coverage };

    // Recall of the part the question did not give away is the spine: an answer
    // that never says the answer cannot be carried by wording alone. Precision,
    // character overlap and adjacency shape what recall has already earned.
    let support = 0.35 * overlap.precision + 0.40 * trigram + 0.25 * overlap.bigram;
    let lexical = overlap.recall * (0.58 + 0.42 * support);

    // Agreeing on every axis both sides spoke on is independent evidence, and it
    // is the only thing that rescues a correct answer with no words in common.
    let axis_support = if axes_shared > 0 && axes_agreed == axes_shared {
        let breadth = if axes_shared >= 2 { 1.0 } else { 0.6 };
        0.30 + 0.30 * breadth
    } else {
        0.0
    };

    let mut content_tokens = 0u32;
    let mut ci = 0usize;
    while ci < ma.count {
        if ma.weight[ci] >= 1.0 {
            content_tokens += 1;
        }
        ci += 1;
    }
    // When the axes carry the answer, the lexical guards are measuring something
    // the score has already decided not to depend on, so they are not applied.
    let carried_by_axes = content_tokens >= 2 && axis_support > lexical;
    let base = if carried_by_axes { axis_support } else { lexical };

    // ---- gates
    let mut penalty = 1.0f32;
    if target_conflict {
        penalty *= 0.05;
    }
    if unknown_gap {
        penalty *= 0.10;
    }
    if verdict_gap >= 1.2 {
        penalty *= 0.04;
    } else if verdict_gap >= 0.6 {
        penalty *= 0.10;
    } else if verdict_gap >= 0.35 {
        penalty *= 0.40;
    } else if verdict_gap >= 0.2 {
        penalty *= 0.75;
    }
    if confirm_gap >= 1.2 {
        penalty *= 0.18;
    } else if confirm_gap >= 0.6 {
        penalty *= 0.50;
    }
    if direction_gap >= 1.2 {
        penalty *= 0.05;
    } else if direction_gap >= 0.6 {
        penalty *= 0.15;
    }
    if affirm_gap >= 1.2 {
        penalty *= 0.05;
    } else if affirm_gap >= 0.6 {
        penalty *= 0.15;
    }
    if real_gap >= 1.2 {
        penalty *= 0.05;
    } else if real_gap >= 0.6 {
        penalty *= 0.15;
    }
    if scale_conflict {
        penalty *= 0.05;
    }
    if conflict > 0.0 {
        penalty *= 0.15 + 0.35 * (1.0 - conflict);
    }
    if slot_conflict > 0.0 {
        penalty *= 1.0 - 0.50 * slot_conflict;
    }
    // An answer that names every candidate has not chosen one.
    if !carried_by_axes && overlap.precision < 0.30 {
        penalty *= 0.55 + 1.5 * overlap.precision;
    }
    let strangers = foreign_names(answer, ma, ground_truth, gt, question, q);
    if strangers > 0 {
        let capped = if strangers > 2 { 2 } else { strangers };
        penalty *= 1.0 - 0.09 * capped as f32;
    }
    // Every word of the ground truth, in a different order, sharing not one of
    // its pairings: "France is the capital of Paris" built from "Paris is the
    // capital of France". Only a near-verbatim rearrangement can trip this.
    if !carried_by_axes
        && overlap.pairs >= 2
        && overlap.recall > 0.90
        && overlap.precision > 0.90
        && trigram > 0.80
        && overlap.bigram <= 0.0
    {
        penalty *= 0.35;
    }


    // An answer that contradicts nothing has passed every falsifiable test this
    // module has: target, verdict, no-verdict, record, direction, yes/no,
    // authenticity, every figure, its scale, its field, adjacency and strangers.
    // At that point the wording decides how far above the bar it sits, not
    // whether it is right. Answers with too little in common with the ground
    // truth to have been tested at all do not qualify, which is what keeps
    // boilerplate and a repeated question out.
    // The floor is earned on the part of the ground truth the question did not
    // already give away. An answer that hands the question back covers the
    // sentence and none of the answer, and does not qualify. Where the question
    // already contains nearly all of the ground truth there is no such part to
    // measure, and overall recall stands in.
    let uncontradicted = penalty >= 0.999
        && base >= 0.18
        && if overlap.novel_share < 0.35 {
            overlap.recall >= 0.30
        } else {
            overlap.novel >= 0.15
        };
    let quality = if uncontradicted {
        0.88 + 0.12 * clamp01(base / 0.60)
    } else {
        let lift = smoothstep(clamp01((base - 0.12) / 0.40));
        0.96 * lift + 0.04 * base
    };

    Eval {
        score: clamp01(penalty * quality),
        base,
        recall: overlap.recall,
        precision: overlap.precision,
        trigram,
        penalty,
        verdict_gap,
        confirm_gap,
        direction_gap,
        affirm_gap,
        axis_support,
        bigram: overlap.bigram,
        real_gap,
        conflict,
        slot_conflict,
        target_conflict: if target_conflict { 1.0 } else { 0.0 },
        unknown_gap: if unknown_gap { 1.0 } else { 0.0 },
    }
}

#[no_mangle]
pub extern "C" fn rank_answer(
    q_ptr: i32,
    q_len: i32,
    gt_ptr: i32,
    gt_len: i32,
    ma_ptr: i32,
    ma_len: i32,
) -> f32 {
    unsafe {
        let value = score(
            read_bytes(q_ptr, q_len),
            read_bytes(gt_ptr, gt_len),
            read_bytes(ma_ptr, ma_len),
        );
        // the node writes the three strings, calls once, and is done with them:
        // handing the next call the same memory keeps the bump pointer from
        // wrapping into a string this call is still reading
        HEAP_OFFSET = 0;
        value
    }
}

/// Development-only view of the parts behind a score. Compiled out of the
/// binary that gets registered: `rank_answer` returns a float and nothing else.
#[cfg(feature = "probe")]
#[no_mangle]
pub extern "C" fn probe(
    field: i32,
    q_ptr: i32,
    q_len: i32,
    gt_ptr: i32,
    gt_len: i32,
    ma_ptr: i32,
    ma_len: i32,
) -> f32 {
    let e = unsafe {
        evaluate(
            read_bytes(q_ptr, q_len),
            read_bytes(gt_ptr, gt_len),
            read_bytes(ma_ptr, ma_len),
        )
    };
    match field {
        1 => e.base,
        2 => e.recall,
        3 => e.precision,
        4 => e.trigram,
        5 => e.penalty,
        7 => e.verdict_gap,
        8 => e.confirm_gap,
        9 => e.conflict,
        10 => e.slot_conflict,
        11 => e.target_conflict,
        12 => e.unknown_gap,
        13 => e.direction_gap,
        14 => e.affirm_gap,
        15 => e.axis_support,
        16 => e.bigram,
        17 => e.real_gap,
        _ => e.score,
    }
}
