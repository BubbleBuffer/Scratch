// Shared parts of media blocks (image, file, bookmark): the caption, which
// is the block's own text and only takes room once it has text or focus,
// and the grey "Add a ..." row shown before the block has content.

import 'package:flutter/widgets.dart';

import '../editor/model.dart';
import '../registry/block_registry.dart';
import '../ui/icon_x.dart';
import '../ui/ui.dart';

/// Caption text: 14/19.6 textSecondary.
TextStyle captionStyle(Node n, AppColors c, TypeScale ty) => tsRaw(14, 19.6, 400, c.textSecondary);

/// [content] with the caption [text] under it.
Widget withCaption(BlockContext b, Widget content, Widget text) {
  final show = b.node.text.isNotEmpty || b.host.isFocused(b.node);
  return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
    GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: () => b.host.selectBlock(b.node),
      onLongPress: b.mobile ? () => b.host.onLongPress(b.node) : null,
      child: content,
    ),
    if (show)
      Padding(padding: const EdgeInsets.only(top: 6), child: text)
    else
      // Kept in the tree (zero height) so the caption can take focus.
      ClipRect(
        child: SizedBox(
          height: 0,
          child: OverflowBox(alignment: Alignment.topLeft, maxHeight: double.infinity, child: text),
        ),
      ),
  ]);
}

/// The grey box with an icon and "Add an image" (and the like).
class EmptyMediaRow extends StatelessWidget {
  const EmptyMediaRow({super.key, required this.icon, required this.label, required this.onTap});
  final Ix icon;
  final String label;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Hover(
      onTap: onTap,
      radius: Radii.sm,
      hoverColor: c.bgPressed,
      child: Container(
        height: 46,
        padding: const EdgeInsets.symmetric(horizontal: 12),
        decoration: BoxDecoration(color: c.bgHover, borderRadius: BorderRadius.circular(Radii.sm)),
        child: Row(children: [
          IconX(icon, size: 22, color: c.iconSecondary),
          const SizedBox(width: 10),
          Flexible(
            child: Text(label,
                maxLines: 1, overflow: TextOverflow.ellipsis, style: ts(context.type.ui, c.textSecondary, weight: 400)),
          ),
        ]),
      ),
    );
  }
}

/// The empty row that opens the block's picker.
Widget emptyMedia(BlockContext b, Ix icon, String label) => Builder(
      builder: (anchor) => EmptyMediaRow(
        key: ValueKey('pick-${b.node.id}'),
        icon: icon,
        label: label,
        onTap: () => b.host.openBlockPicker(b.node, anchor),
      ),
    );
