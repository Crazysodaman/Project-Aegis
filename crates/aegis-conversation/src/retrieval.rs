//! Provider-independent memory retrieval.
//!
//! The default retriever is explicit keyword matching. It does not claim
//! semantic or embedding-based relevance. A future semantic retriever can
//! implement this trait without changing the conversation engine.

use std::collections::BTreeSet;

use aegis_core_sdk::{CoreMemoryStore, MemoryEntry, MemoryQuery, SdkError};

/// Strategy used to retrieve private memories for context assembly.
pub trait MemoryRetriever: std::fmt::Debug {
    /// Honest name of the retrieval strategy, for diagnostics and UI.
    fn name(&self) -> &'static str;

    /// Whether this retriever provides semantic ranking.
    fn semantic(&self) -> bool {
        false
    }

    /// Retrieve up to `limit` memories relevant to `text`.
    fn retrieve(
        &self,
        memory: &CoreMemoryStore,
        text: &str,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, SdkError>;

    /// Retrieve with an explicit per-call method description. Strategies that
    /// can fall back at runtime override this to report what actually happened.
    fn retrieve_with_meta(
        &self,
        memory: &CoreMemoryStore,
        text: &str,
        limit: usize,
    ) -> Result<RetrievalOutcome, SdkError> {
        Ok(RetrievalOutcome {
            entries: self.retrieve(memory, text, limit)?,
            method: self.name().to_owned(),
            semantic: self.semantic(),
        })
    }
}

/// Result of one retrieval call, including the method actually used.
#[derive(Debug, Clone, PartialEq)]
pub struct RetrievalOutcome {
    pub entries: Vec<MemoryEntry>,
    pub method: String,
    pub semantic: bool,
}

/// Local keyword retriever used when no semantic provider is configured.
///
/// The retriever tokenizes the user turn into distinct terms (three or more
/// characters), searches each term, and deduplicates by memory id.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeywordRetriever;

impl KeywordRetriever {
    pub const NAME: &'static str = "keyword";
}

impl MemoryRetriever for KeywordRetriever {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn retrieve(
        &self,
        memory: &CoreMemoryStore,
        text: &str,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, SdkError> {
        let limit = limit.min(50);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
        let mut seen = BTreeSet::new();
        for term in search_terms(text, 8) {
            let query = MemoryQuery::new().with_text(term).with_limit(limit);
            for entry in memory.search(&query)? {
                if seen.insert(entry.id.clone()) {
                    entries.push(entry);
                    if entries.len() >= limit {
                        return Ok(entries);
                    }
                }
            }
        }
        Ok(entries)
    }
}

fn search_terms(text: &str, maximum: usize) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_alphanumeric() {
            current.push(character.to_ascii_lowercase());
        } else if !current.is_empty() {
            push_term(&mut terms, &mut current, maximum);
        }
        if terms.len() >= maximum {
            return terms;
        }
    }
    if !current.is_empty() {
        push_term(&mut terms, &mut current, maximum);
    }
    terms
}

fn push_term(terms: &mut Vec<String>, current: &mut String, maximum: usize) {
    if current.chars().count() >= 3 && !terms.contains(current) && terms.len() < maximum {
        terms.push(current.clone());
    }
    current.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_are_distinct_and_bounded() {
        let terms = search_terms("The owner prefers tea, and tea is warm.", 8);
        assert_eq!(terms, vec!["the", "owner", "prefers", "tea", "and", "warm"]);
        assert!(search_terms("a b c", 8).is_empty());
    }
}
