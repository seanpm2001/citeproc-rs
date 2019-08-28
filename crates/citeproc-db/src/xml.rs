// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
//
// Copyright © 2019 Corporation for Digital Scholarship

use std::io;
use std::str::FromStr;
use std::sync::Arc;

use csl::{
    locale::{Lang, Locale, LocaleOptions, LocaleSource},
    style::{Name, Style},
};
use fnv::FnvHashSet;

pub trait HasFetcher {
    fn get_fetcher(&self) -> Arc<dyn LocaleFetcher>;
}

/// Salsa interface to a CSL style.
#[salsa::query_group(StyleDatabaseStorage)]
pub trait StyleDatabase {
    #[salsa::input]
    fn style(&self) -> Arc<Style>;

    /// Grabs the Name options from `<style>` + `<citation>` elements
    fn name_citation(&self) -> Arc<Name>;
}

fn name_citation(db: &impl StyleDatabase) -> Arc<Name> {
    let style = db.style();
    let default = Name::root_default();
    let root = &style.name_inheritance;
    let citation = &style.citation.name_inheritance;
    Arc::new(default.merge(root).merge(citation))
}

/// Salsa interface to locales, including merging.
#[salsa::query_group(LocaleDatabaseStorage)]
pub trait LocaleDatabase: salsa::Database + StyleDatabase + HasFetcher {
    #[salsa::input]
    fn locale_input_xml(&self, key: Lang) -> Arc<String>;
    #[salsa::input]
    fn locale_input_langs(&self) -> Arc<FnvHashSet<Lang>>;

    /// Backed by the LocaleFetcher implementation
    fn locale_xml(&self, key: Lang) -> Option<Arc<String>>;

    /// Derived from a `Style`
    fn inline_locale(&self, key: Option<Lang>) -> Option<Arc<Locale>>;

    /// A locale object, which may be `Default::default()`
    fn locale(&self, key: LocaleSource) -> Option<Arc<Locale>>;

    /// Derives the full lang inheritance chain, and merges them into one
    fn merged_locale(&self, key: Lang) -> Arc<Locale>;

    /// Even though we already have a merged `LocaleOptionsNode` struct, all its fields are
    /// `Option`. To avoid having to unwrap each field later on, we merge whatever options did
    /// get provided into a non-`Option` defaults struct.
    fn locale_options(&self, key: Lang) -> Arc<LocaleOptions>;

    fn default_locale(&self) -> Arc<Locale>;
}

fn default_locale(db: &impl LocaleDatabase) -> Arc<Locale> {
    db.merged_locale(db.style().default_locale.clone())
}

fn locale_xml(db: &impl LocaleDatabase, key: Lang) -> Option<Arc<String>> {
    let stored = db.locale_input_langs();
    if stored.contains(&key) {
        return Some(db.locale_input_xml(key));
    }
    debug!("fetching locale: {:?}", key);
    match db.get_fetcher().fetch_string(&key) {
        Ok(Some(s)) => Some(Arc::new(s)),
        Ok(None) => None,
        Err(e) => {
            error!("{:?}", e);
            None
        }
    }
}

fn inline_locale(db: &impl LocaleDatabase, key: Option<Lang>) -> Option<Arc<Locale>> {
    db.style().locale_overrides.get(&key).cloned().map(Arc::new)
}

fn locale(db: &impl LocaleDatabase, key: LocaleSource) -> Option<Arc<Locale>> {
    match key {
        LocaleSource::File(ref lang) => {
            let string = db.locale_xml(lang.clone());
            string
                .and_then(|s| match Locale::from_str(&s) {
                    Ok(l) => Some(l),
                    Err(e) => {
                        error!("failed to parse locale for lang {}: {:?}", lang, e);
                        None
                    }
                })
                .map(Arc::new)
        }
        LocaleSource::Inline(ref lang) => db.inline_locale(lang.clone()),
    }
}

fn merged_locale(db: &impl LocaleDatabase, key: Lang) -> Arc<Locale> {
    debug!("requested locale {:?}", key);
    let locales = key
        .iter()
        .filter_map(|src| db.locale(src))
        .collect::<Vec<_>>();
    Arc::new(
        locales
            .into_iter()
            .rev()
            .fold(None, |mut acc, l| match acc {
                None => Some((*l).clone()),
                Some(ref mut base) => {
                    debug!("merging locales: {:?} <- {:?}", base.lang, l.lang);
                    base.merge(&l);
                    acc
                }
            })
            .unwrap_or_else(Locale::default),
    )
}

fn locale_options(db: &impl LocaleDatabase, key: Lang) -> Arc<LocaleOptions> {
    let merged = &db.merged_locale(key).options_node;
    Arc::new(LocaleOptions::from_merged(merged))
}

cfg_if::cfg_if! {
    if #[cfg(feature = "parallel")] {
        pub trait LocaleFetcher: Send + Sync {
            fn fetch_string(&self, lang: &Lang) -> Result<Option<String>, LocaleFetchError>;
        }
    } else {
        pub trait LocaleFetcher {
            fn fetch_string(&self, lang: &Lang) -> Result<Option<String>, LocaleFetchError>;
        }
    }
}

#[derive(Debug)]
pub enum LocaleFetchError {
    Io(io::Error),
    Other(String),
}

impl From<String> for LocaleFetchError {
    fn from(err: String) -> LocaleFetchError {
        LocaleFetchError::Other(err)
    }
}

impl From<io::Error> for LocaleFetchError {
    fn from(err: io::Error) -> LocaleFetchError {
        LocaleFetchError::Io(err)
    }
}

use std::collections::HashMap;

pub struct PredefinedLocales(pub HashMap<Lang, String>);

impl LocaleFetcher for PredefinedLocales {
    fn fetch_string(&self, lang: &Lang) -> Result<Option<String>, LocaleFetchError> {
        Ok(self.0.get(lang).cloned())
    }
}
