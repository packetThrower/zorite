---
title: Password & encryption
description: 'Encrypt the whole database with a password — an unlock screen at launch, keychain remember, idle auto-lock, and what is (and is not) stored.'
---

Zorite can encrypt your entire database with a password. This is real
encryption at rest (SQLCipher, AES-256), not a cosmetic gate: without the
password, the `zorite.db` file on disk is unreadable bytes.

## Setting a password

**Settings → Security → Set password…** encrypts the database in place.
From then on Zorite shows an unlock screen at launch; your notes, whiteboards,
search index, and settings are all inside the encrypted file.

**Change password…** and **Remove password…** live in the same card — both
ask for the current password first. Removing decrypts the file back to a
plain SQLite database.

:::caution[There is no recovery]
The password is never written anywhere, and there is no backdoor. A forgotten
password means the data in the encrypted file is gone. Consider the
**Remember on this device** option below, or keep the password in a password
manager.
:::

## The unlock screen

An encrypted database boots to a small unlock window. Type the password and
press Enter (or the **Unlock** button); a wrong password clears the field and
lets you retry. The window follows your saved theme when it can, and falls
back to the default look — the theme settings live inside the lock too.

## Remember on this device

Checking **Remember on this device** (on the unlock screen or in Settings →
Security) stores the password in the operating system's credential store:

| Platform | Store | Lifetime |
| --- | --- | --- |
| macOS | Keychain | until you turn it off |
| Windows | Credential Manager | until you turn it off |
| Linux | kernel keyring (keyutils) | until reboot/logout |

With a remembered password Zorite unlocks itself at launch. Zorite only
touches the credential store at all after you opt in, and turning the toggle
off deletes the stored entry.

## Auto-lock

**Settings → Security → Auto-lock** locks Zorite after a period of
inactivity (5 minutes to 1 hour), closing every window and returning to the
unlock screen. **Lock now** does the same immediately. Re-unlocking after a
lock always requires typing the password — the remembered credential is only
consulted at launch.

## What's stored where

- **The password itself: nowhere.** SQLCipher derives the encryption key from
  your passphrase at each unlock (PBKDF2). The only copy that ever persists
  is the optional credential-store entry above, on your explicit opt-in.
- **While unlocked**, the key lives in the app's memory and is wiped when you
  lock or quit.
- **Earlier backups stay as they were.** Pre-migration snapshots
  (`zorite.db.bak-*`) made *before* you set a password are still plaintext
  until you delete them; snapshots made after are encrypted like the main
  file.

## Interactions worth knowing

- **Data location moves** (Settings → General) copy the encrypted file as-is
  — the password travels with it.
- **Schema migrations** on app updates run after unlock, inside the
  encrypted file, with the same pre-migration snapshot safety net.
- A corrupt-file recovery flow can't mistake an encrypted database for a
  damaged one — encryption is detected before any recovery is attempted.
