// The block registry. Each block type is one file in lib/blocks/ that
// describes itself with a [BlockType] and registers it here; the editor, the
// `/` menu, "Turn into", markdown shortcuts and the block sheet only ever go
// through this registry. Kinds the app does not know (the core has more
// kinds than the app renders) get a read-only placeholder and are kept
// intact: the editor never sends ops for them unless the user moves or
// deletes the block.

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../editor/model.dart';
import '../ui/icon_x.dart';
import '../ui/ui.dart';
import '../ui/unsupported_card.dart';
import 'block_codecs.dart';

/// Blocks reach add-ons through this registry, so its node type does too.
export '../editor/model.dart' show Node;

/// What a block's widgets can ask of the editor that shows them.
abstract interface class BlockHost {
  void toggleChecked(Node n);
  void selectBlock(Node n);
  void onLongPress(Node n);

  /// Toggles: whether [n] shows its children, and flipping that.
  bool isOpen(Node n);
  void toggleOpen(Node n);

  /// Adds an empty paragraph as the first child of [n] and focuses it.
  void addChild(Node n);

  /// Sets a block's target (callout icon, image attachment, bookmark URL).
  void setTarget(Node n, String? target);

  /// Sets an image's width in percent of its column (null: full width).
  void setWidth(Node n, int? width);

  /// Replaces a block's text (captions typed outside the block's own field).
  void setText(Node n, String text);

  /// Turns [paragraph] into a block of [type]. [data] gives the new block's
  /// fields by name; `text` is read, and the paragraph's marks are dropped.
  /// One undo step.
  void replaceWithBlock(Node paragraph, String type, Map<String, Object?> data);

  /// Inserts a block of [type] after [paragraph], with [data] as in
  /// [replaceWithBlock]. [removeText], when given, is cut from the paragraph
  /// first, with the marks that cover it. One undo step.
  void insertBlockAfter(
    Node paragraph,
    String type,
    Map<String, Object?> data, {
    TextRange? removeText,
  });

  /// Turns [block] into a paragraph holding [text] with [marks] (for example
  /// an inline chip over its text). One undo step.
  void replaceWithParagraph(Node block, String text, List<BMark> marks);

  /// The rows of [n]'s descendants for a block that lays out its own
  /// children (a column): text [width] wide after a [gutter] for handles.
  Widget nestedBlocks(Node n, double width, double gutter);

  /// Opens a page (link to page, sub-page, mention).
  void openPage(String id);

  /// The page's blocks in reading order (table of contents).
  List<Node> get readingOrder;

  /// Scrolls to and focuses [n] (table of contents).
  void revealBlock(Node n);

  /// Opens the emoji picker for a callout's icon, next to [anchor].
  void openEmojiPicker(Node n, BuildContext anchor);

  /// Opens the picker that fills a media block (image, file, bookmark,
  /// link to page), next to [anchor].
  void openBlockPicker(Node n, BuildContext anchor);

  /// Focuses [n]'s text (a media block's caption).
  void focusCaption(Node n);

  /// Whether [n] is focused.
  bool isFocused(Node n);

  /// The id of the page being edited.
  String get pageId;
}

/// An editor that lets a block's widgets add and remove whole blocks (view
/// blocks add and delete their sibling views). Read-only hosts are not.
abstract interface class BlockEditingHost implements BlockHost {
  /// The blocks at [n]'s level, [n] among them, in order.
  List<Node> siblingsOf(Node n);

  /// Inserts a [kind] block holding [text] (and [target]) right after
  /// [after] and returns its id. One undo step.
  String insertBlock(Node after, String kind, String text, {String? target});

  /// Deletes [n] and its children. One undo step.
  void deleteBlock(Node n);
}

/// A host rendering a document preview without storage mutations.
abstract interface class ReadOnlyBlockHost implements BlockHost {}

/// How a block's children show in the page.
enum ChildMode {
  /// Indented rows under the block (lists, quotes, paragraphs).
  list,

  /// Indented rows while the block is open (toggles).
  toggle,

  /// Laid out by the block's own [BlockType.body] (column lists).
  embedded,
}

/// Everything a block type needs to draw one block.
class BlockContext {
  const BlockContext({
    required this.context,
    required this.node,
    required this.host,
    required this.style,
    required this.lineHeight,
  });
  final BuildContext context;
  final Node node;
  final BlockHost host;

  /// The block's text style (from [BlockType.textStyle]).
  final TextStyle style;

  /// Height of one line of [style].
  final double lineHeight;

  AppColors get colors => context.colors;
  TypeScale get type => context.type;
  bool get mobile => context.mobile;
  bool get readOnly => host is ReadOnlyBlockHost;
}

TextStyle _bodyStyle(Node n, AppColors c, TypeScale ty) =>
    ts(ty.body, c.textPrimary);
double _noMargin(bool mobile) => 0;

class BlockType {
  const BlockType({
    required this.kind,
    required this.label,
    this.hint = '',
    this.icon,
    this.iconX,
    this.iconText,
    this.aliases = '',
    this.shortcut = '',
    this.markdown = const [],
    this.editable = true,
    this.listItem = false,
    this.exitOnEmptyEnter = false,
    this.multiline = false,
    this.acceptsChildren = true,
    this.inMenu = true,
    this.placeholder = '',
    this.alwaysShowPlaceholder = false,
    this.textStyle = _bodyStyle,
    this.topMargin = _noMargin,
    this.decorate,
    this.body,
    this.handleTop,
    this.children = ChildMode.list,
    this.colorable = true,
    this.slashLabel,
    this.paintsColor = false,
    this.atomic = false,
    this.group = 'Basic blocks',
    this.insertOnly = false,
    this.insertKind,
    this.createTarget,
    this.codecs,
  });

  /// The core's kind name.
  final String kind;

  /// How the block writes as markdown, on the clipboard and as plain text.
  /// Null: [BlockRegistry.codecsOf] uses the legacy table, else the generic
  /// fallback. Contract tests flag a new kind that has neither.
  final BlockCodecs? codecs;

  /// Name in menus ("Heading 1").
  final String label;

  /// One-line description (menus with descriptions).
  final String hint;

  /// Icon in menus; [iconText] ("H1") when there is no fitting icon.
  final Ico? icon;
  final Ix? iconX;
  final String? iconText;

  /// Extra words the `/` filter matches, separated by spaces.
  final String aliases;

  /// The markdown shortcut shown in the `/` menu ("#", "-", "```").
  final String shortcut;

  /// Prefixes that turn a paragraph into this kind when typed at its start
  /// ("# ", "- ", "```").
  final List<String> markdown;

  /// Has editable text. Non-editable blocks draw [body] instead.
  final bool editable;

  /// A list item: Enter continues the list with a new item of this kind;
  /// Enter on an empty item outdents it (or ends the list).
  final bool listItem;

  /// Enter on an empty block turns it into a paragraph (quotes, lists).
  final bool exitOnEmptyEnter;

  /// Enter inserts a line break instead of splitting (code).
  final bool multiline;

  /// Blocks can be indented under this one.
  final bool acceptsChildren;

  /// Offered in the `/` menu and "Turn into".
  final bool inMenu;

  /// Shown in an empty block (when focused, or always if
  /// [alwaysShowPlaceholder]).
  final String placeholder;
  final bool alwaysShowPlaceholder;

  final TextStyle Function(Node n, AppColors c, TypeScale ty) textStyle;

  /// Extra space above the block (headings).
  final double Function(bool mobile) topMargin;

  /// Wraps the block's text (list markers, quote bar, code box).
  final Widget Function(BlockContext b, Widget text)? decorate;

  /// The whole block, for non-editable types (divider, placeholder).
  final Widget Function(BlockContext b)? body;

  /// Vertical offset of the desktop "+ ⋮⋮" handles (default: centred on the
  /// first line).
  final double Function(BlockContext b)? handleTop;

  /// How the block's children show.
  final ChildMode children;

  /// Offered in the "Color" menu.
  final bool colorable;

  /// Name in the `/` menu when it differs from [label] ("2 columns").
  final String? slashLabel;

  String get menuLabel => slashLabel ?? label;

  /// Draws its own background colour (callouts); otherwise the row fills
  /// a background colour behind the block.
  final bool paintsColor;

  /// A media block (image, file, bookmark): its text is a caption, shown
  /// under the content; Backspace in an empty caption selects the block
  /// instead of turning it into text.
  final bool atomic;

  /// The `/` menu section ("Basic blocks", "Media", "Advanced").
  final String group;

  /// A `/` menu entry that inserts something (a sub-page, columns) rather
  /// than a kind a block can be turned into.
  final bool insertOnly;

  /// A compatibility menu entry may insert a newer storage kind.
  final String? insertKind;

  /// Creates the target of a newly inserted block (for example a database).
  final Future<String?> Function(BuildContext context, String pageId)?
  createTarget;

  /// This type under another `/` menu section.
  BlockType inGroup(
    String group, {
    BlockCodecs? codecs,
    Widget Function(BlockContext)? body,
  }) => BlockType(
    kind: kind,
    label: label,
    hint: hint,
    icon: icon,
    iconX: iconX,
    iconText: iconText,
    aliases: aliases,
    shortcut: shortcut,
    markdown: markdown,
    editable: editable,
    listItem: listItem,
    exitOnEmptyEnter: exitOnEmptyEnter,
    multiline: multiline,
    acceptsChildren: acceptsChildren,
    inMenu: inMenu,
    placeholder: placeholder,
    alwaysShowPlaceholder: alwaysShowPlaceholder,
    textStyle: textStyle,
    topMargin: topMargin,
    decorate: decorate,
    body: body ?? this.body,
    handleTop: handleTop,
    children: children,
    colorable: colorable,
    slashLabel: slashLabel,
    paintsColor: paintsColor,
    atomic: atomic,
    group: group,
    insertOnly: insertOnly,
    insertKind: insertKind,
    createTarget: createTarget,
    codecs: codecs ?? this.codecs,
  );

  bool matches(String query) {
    final q = query.trim().toLowerCase();
    if (q.isEmpty) return true;
    return menuLabel.toLowerCase().contains(q) ||
        aliases.split(' ').any((w) => w.startsWith(q));
  }
}

class BlockRegistry {
  final _types = <String, BlockType>{};
  final _unknown = <String, BlockType>{};

  /// Adds or replaces a type. Menu order is registration order.
  void register(BlockType t, {String? before}) {
    _types[t.kind] = t;
    if (before != null && before != t.kind && _types.containsKey(before)) {
      final entries = _types.entries.toList();
      _types.clear();
      for (final entry in entries) {
        if (entry.key == t.kind) continue;
        if (entry.key == before) _types[t.kind] = t;
        _types[entry.key] = entry.value;
      }
    }
  }

  bool has(String kind) => _types.containsKey(kind);

  /// The type for [kind]; a read-only placeholder when none is registered.
  BlockType of(String kind) =>
      _types[kind] ?? _unknown.putIfAbsent(kind, () => unsupportedBlock(kind));

  Iterable<BlockType> get all => _types.values;

  /// The text codecs for [kind]: its registration's, else the legacy table's,
  /// else the explicit generic fallback (flagged `generic`).
  BlockCodecs codecsOf(String kind) =>
      _types[kind]?.codecs ?? legacyBlockCodecs(kind) ?? genericBlockCodecs;

  /// Registered kinds with no codecs of their own and no legacy entry: they
  /// would write generically without anyone deciding so. Contract tests
  /// require this to be empty.
  List<String> get incompleteCodecKinds => [
    for (final t in _types.values)
      if (t.codecs == null && legacyBlockCodecs(t.kind) == null) t.kind,
  ];

  /// Types offered in the `/` menu and "Turn into", in order.
  List<BlockType> get menuTypes => [
    for (final t in _types.values)
      if (t.inMenu) t,
  ];

  /// Types a block can be turned into ("Turn into").
  List<BlockType> get turnIntoTypes => [
    for (final t in menuTypes)
      if (!t.insertOnly && !t.atomic) t,
  ];

  /// The `/` menu section of filtered entries.
  static const resultsGroup = 'Results';

  /// Menu types matching the `/` query. While filtering they form one
  /// "Results" section rather than their usual sections.
  List<BlockType> filter(String query) {
    final filtering = query.trim().isNotEmpty;
    return [
      for (final t in menuTypes)
        if (t.matches(query)) filtering ? t.inGroup(resultsGroup) : t,
    ];
  }

  /// The type whose markdown prefix is exactly [text] up to [caret].
  (BlockType, String)? markdownAt(String text, int caret) {
    for (final t in _types.values) {
      for (final p in t.markdown) {
        if (caret == p.length && text.startsWith(p)) return (t, p);
      }
    }
    return null;
  }
}

/// The app's block registry (filled by `registerBuiltins`).
final blockRegistry = BlockRegistry();

/// The placeholder for kinds without a registered type: a read-only
/// "Not supported on this version" card (see [UnsupportedCard]).
BlockType unsupportedBlock(String kind) => BlockType(
  kind: kind,
  label: 'Unsupported block',
  editable: false,
  inMenu: false,
  colorable: false,
  body: (b) => UnsupportedCard(kind: kind, text: b.node.text),
);
