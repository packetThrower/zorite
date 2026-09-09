---
title: Language
description: 'Zorite is available in English and Simplified Chinese; pick the language under Settings → General, or contribute a new one — translations are plain YAML files.'
---

Zorite's interface is available in **English** and **简体中文 (Simplified
Chinese)**. Pick one under **Settings → General → Language**; the app switches
immediately — menus, settings, dialogs, the editor and reading-view context
menus, the `/` command palette, tab titles, and the unlock screen included.
The Chinese localization was contributed by the community
([@shimoxi123](https://github.com/shimoxi123)) — thank you!

A few things stay language-independent on purpose:

- **Your notes** are never touched; only the app's own text changes.
- **Settings search** still matches English synonyms, so typing `theme` finds
  the appearance card in any language.
- **Slash commands** match both the localized label and the English name, so
  `/table` works everywhere.

## Right-to-left languages

Notes written in Arabic, Hebrew, or Persian lay out correctly regardless of
the interface language — see [Right-to-left text](/zorite/usage/journal/#right-to-left-text).
The interface itself is not yet offered in a right-to-left language; the
sidebar can be docked on the right (Settings → Appearance) if that suits your
reading direction better.

## Adding a language

Translations live in the repository as one YAML file per language
(`locales/en.yml`, `locales/zh-CN.yml`), loaded by
[`rust-i18n`](https://crates.io/crates/rust-i18n). To add one, copy `en.yml` to
`locales/<code>.yml`, translate the values (keys stay as they are), and open a
pull request — no code changes are needed for the strings themselves, though
a new language should be checked in the app for text that no longer fits its
control (Chinese ran narrower than English; a long Latin language may not).
