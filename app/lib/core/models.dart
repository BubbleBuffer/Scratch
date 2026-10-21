// The data types the app shares with the core (generated from
// app/rust/src/api/types.rs), plus small helpers. Import this rather than
// the generated files.
//
// - Mark offsets are UTF-16 code units (Dart string indices).
// - Block kinds are the core's names (see [Kinds]); the block registry is
//   keyed by them.

import '../src/rust/api/types.dart';

export '../src/rust/api/sync.dart' show NotesHub, NotesSync, defaultSyncOptions;
export '../src/rust/api/types.dart';
export '../src/rust/api/vault.dart' show NotesVault, newId;

/// The core's page types (`PAGE_TYPE_DOCUMENT` and `PAGE_TYPE_DATABASE`). A
/// page without a stored type is a document; other types a mod defines are
/// kept as they are, so copy `pageType` strings rather than a bool.
const documentType = 'document';
const databaseType = 'database';

extension PageRowType on PageRow {
  /// Whether the page is a database (its children are rows).
  bool get isDatabase => pageType == databaseType;
}

extension PageDataType on PageData {
  /// Whether the page is a database (its children are rows).
  bool get isDatabase => pageType == databaseType;
}

/// The core's block kind names.
abstract final class Kinds {
  static const paragraph = 'paragraph';
  static const heading1 = 'heading1';
  static const heading2 = 'heading2';
  static const heading3 = 'heading3';
  static const bulleted = 'bulleted';
  static const numbered = 'numbered';
  static const todo = 'todo';
  static const quote = 'quote';
  static const code = 'code';
  static const divider = 'divider';
  static const linkToPage = 'linktopage';
  static const image = 'image';
  static const file = 'file';
  static const bookmark = 'bookmark';
  static const toggle = 'toggle';
  static const toggleHeading1 = 'toggleheading1';
  static const toggleHeading2 = 'toggleheading2';
  static const toggleHeading3 = 'toggleheading3';
  static const callout = 'callout';
  static const columnList = 'columnlist';
  static const column = 'column';
  static const toc = 'toc';

  /// A GFM pipe table; its text is the table's markdown, verbatim.
  static const table = 'table';

  /// A reference to a chat's chapter or section (JSON in the block's target).
  /// The core's `BlockKind::ChatRef`.
  static const chatRef = 'chat_ref';
}
