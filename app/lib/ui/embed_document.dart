import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../editor/model.dart';
import '../editor/rich_controller.dart' show markedSpan;
import '../registry/block_registry.dart';
import 'block_colors.dart';
import 'ui.dart';

/// A document body using installed block presentations, without an editor,
/// focus ownership or vault mutation. Nested embeds inherit their ancestry.
class EmbedDocument extends StatefulWidget {
  const EmbedDocument({
    super.key,
    required this.page,
    this.registry,
    this.onOpenPage,
  });
  final PageData page;
  final BlockRegistry? registry;
  final ValueChanged<String>? onOpenPage;
  @override
  State<EmbedDocument> createState() => _EmbedDocumentState();
}

class _EmbedDocumentState extends State<EmbedDocument>
    implements ReadOnlyBlockHost {
  late Doc _doc = Doc.fromPage(widget.page);
  final _closed = <String>{};
  final _blockKeys = <String, GlobalKey>{};
  BlockRegistry get _registry => widget.registry ?? blockRegistry;

  @override
  void didUpdateWidget(EmbedDocument oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.page != widget.page) _doc = Doc.fromPage(widget.page);
  }

  @override
  Widget build(BuildContext context) => _rows(_doc.roots);

  Widget _rows(List<Node> nodes) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [for (final node in nodes) _row(node)],
  );

  Widget _row(Node node) => Builder(
    builder: (context) {
      final type = _registry.of(node.kind);
      var style = type.textStyle(node, context.colors, context.type);
      final color = type.colorable && !type.paintsColor ? node.color : null;
      final textColor = paletteText(color, context.colors);
      if (textColor != null) style = style.copyWith(color: textColor);
      final b = BlockContext(
        context: context,
        node: node,
        host: this,
        style: style,
        lineHeight: (style.height ?? 1.5) * (style.fontSize ?? 16),
      );
      Widget body;
      if (type.editable) {
        final text = Text.rich(
          markedSpan(
            node.text,
            node.marks,
            context.colors,
            style,
            textScaler: MediaQuery.textScalerOf(context),
          ),
        );
        body = Padding(
          padding: EdgeInsets.symmetric(vertical: context.mobile ? 3 : 2),
          child: type.decorate?.call(b, text) ?? text,
        );
      } else if ((node.target == null || node.target!.isEmpty) &&
          (type.createTarget != null || type.atomic)) {
        // Some legacy bodies create a target while building. A read-only
        // presentation must never create missing pages or launch a picker.
        body = Text(
          '${type.label} unavailable',
          style: ts(context.type.ui, context.colors.textTertiary),
        );
      } else {
        body = type.body?.call(b) ?? const SizedBox(height: 24);
      }
      return Column(
        key: ValueKey('embed-block-${node.id}'),
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Container(
            key: _blockKeys.putIfAbsent(node.id, GlobalKey.new),
            margin: EdgeInsets.only(
              top: type.topMargin(context.mobile) + 2,
              bottom: 2,
            ),
            padding: const EdgeInsets.symmetric(
              horizontal: Layout.blockPaddingX,
            ),
            decoration: BoxDecoration(
              color: isBackgroundColor(color)
                  ? paletteBackground(color, context.colors)
                  : null,
              borderRadius: BorderRadius.circular(Radii.sm),
            ),
            child: body,
          ),
          if (type.children != ChildMode.embedded &&
              (type.children != ChildMode.toggle || isOpen(node)))
            Padding(
              padding: const EdgeInsets.only(left: Layout.indent),
              child: _rows(node.children),
            ),
        ],
      );
    },
  );

  @override
  String get pageId => _doc.pageId;
  @override
  List<Node> get readingOrder => [
    for (final node in _doc.flat)
      if (!_hasClosedAncestor(node)) node,
  ];
  bool _hasClosedAncestor(Node n) {
    for (var p = n.parent; p != null; p = p.parent) {
      if (_registry.of(p.kind).children == ChildMode.toggle && !isOpen(p)) {
        return true;
      }
    }
    return false;
  }

  @override
  bool isOpen(Node n) => !_closed.contains(n.id);
  @override
  void toggleOpen(Node n) => setState(() {
    if (!_closed.add(n.id)) _closed.remove(n.id);
  });
  @override
  Widget nestedBlocks(Node n, double width, double gutter) => SizedBox(
    width: width + gutter,
    child: Padding(
      padding: EdgeInsets.only(left: gutter),
      child: _rows(n.children),
    ),
  );
  @override
  void openPage(String id) => widget.onOpenPage?.call(id);
  @override
  bool isFocused(Node n) => false;
  @override
  void toggleChecked(Node n) {}
  @override
  void selectBlock(Node n) {}
  @override
  void onLongPress(Node n) {}
  @override
  void addChild(Node n) {}
  @override
  void setTarget(Node n, String? target) {}
  @override
  void setWidth(Node n, int? width) {}
  @override
  void setText(Node n, String text) {}
  @override
  void replaceWithBlock(Node paragraph, String type, Map<String, Object?> data) {}
  @override
  void insertBlockAfter(
    Node paragraph,
    String type,
    Map<String, Object?> data, {
    TextRange? removeText,
  }) {}
  @override
  void replaceWithParagraph(Node block, String text, List<BMark> marks) {}
  @override
  void revealBlock(Node n) {
    final target = _blockKeys[n.id]?.currentContext;
    if (target != null) Scrollable.ensureVisible(target);
  }

  @override
  void openEmojiPicker(Node n, BuildContext anchor) {}
  @override
  void openBlockPicker(Node n, BuildContext anchor) {}
  @override
  void focusCaption(Node n) {}
}
