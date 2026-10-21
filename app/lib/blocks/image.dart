// Image: an attachment (the block's target) with rounded corners and an
// optional caption. On desktop, hovering shows a handle on each side;
// dragging one resizes the image symmetrically (stored as a width in
// percent of the column). A resized image is centred.

import 'dart:io';

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../registry/block_registry.dart';
import '../ui/icon_x.dart';
import '../ui/ui.dart';
import 'attachment.dart';
import 'media.dart';

Widget _decorate(BlockContext b, Widget text) {
  final t = b.node.target;
  final content = t == null || t.isEmpty
      ? emptyMedia(b, Ix.image, 'Add an image')
      : _ImageView(key: ValueKey('image-${b.node.id}'), b: b, id: t);
  return withCaption(b, content, text);
}

class _ImageView extends StatefulWidget {
  const _ImageView({super.key, required this.b, required this.id});
  final BlockContext b;
  final String id;

  @override
  State<_ImageView> createState() => _ImageViewState();
}

class _ImageViewState extends State<_ImageView> {
  bool _hover = false;

  /// Width in pixels while dragging a handle.
  double? _drag;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final n = widget.b.node;
    return LayoutBuilder(builder: (context, box) {
      final full = box.maxWidth;
      final w = (_drag ?? (n.width == null ? full : full * n.width! / 100)).clamp(48.0, full);
      final image = AttachmentBuilder(
        id: widget.id,
        builder: (context, a, loading) {
          if (a != null && a.path.isNotEmpty) {
            return Image.file(
              File(a.path),
              width: w,
              fit: BoxFit.fitWidth,
              filterQuality: FilterQuality.medium,
              errorBuilder: (_, _, _) => _missing(context, w),
            );
          }
          if (loading) return Container(width: w, height: 160, color: c.bgSecondary);
          return _missing(context, w);
        },
      );
      final handles = !context.mobile && (_hover || _drag != null);
      return MouseRegion(
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: Center(
          child: SizedBox(
            width: w,
            child: Stack(children: [
              ClipRRect(borderRadius: BorderRadius.circular(Radii.lg), child: image),
              if (handles) ...[
                _handle(left: true, full: full, w: w),
                _handle(left: false, full: full, w: w),
              ],
            ]),
          ),
        ),
      );
    });
  }

  Widget _missing(BuildContext context, double w) {
    final c = context.colors;
    return Container(
      width: w,
      height: 120,
      alignment: Alignment.center,
      color: c.bgSecondary,
      child: Row(mainAxisSize: MainAxisSize.min, children: [
        IconX(Ix.image, size: 20, color: c.iconTertiary),
        const SizedBox(width: 8),
        Text('Image not available on this device', style: ts(context.type.ui, c.textTertiary, weight: 400)),
      ]),
    );
  }

  Widget _handle({required bool left, required double full, required double w}) {
    final c = context.colors;
    return Positioned(
      left: left ? 0 : null,
      right: left ? null : 0,
      top: 0,
      bottom: 0,
      width: 18,
      child: MouseRegion(
        cursor: SystemMouseCursors.resizeColumn,
        child: GestureDetector(
          key: ValueKey('image-resize-${left ? 'l' : 'r'}-${widget.b.node.id}'),
          behavior: HitTestBehavior.opaque,
          onHorizontalDragStart: (_) => setState(() => _drag = w),
          onHorizontalDragUpdate: (d) => setState(() {
            // Symmetric: both edges move by the drag.
            final dx = left ? -d.delta.dx : d.delta.dx;
            _drag = ((_drag ?? w) + 2 * dx).clamp(48.0, full);
          }),
          onHorizontalDragEnd: (_) {
            final pct = ((_drag ?? w) / full * 100).round();
            setState(() => _drag = null);
            widget.b.host.setWidth(widget.b.node, pct >= 99 ? null : pct);
          },
          child: Center(
            child: Container(
              width: 6,
              height: 48,
              decoration: BoxDecoration(
                color: const Color(0x99000000),
                borderRadius: BorderRadius.circular(20),
                border: Border.all(color: c.bgPrimary.withValues(alpha: 0.9)),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

const imageBlock = BlockType(
  kind: Kinds.image,
  label: 'Image',
  hint: 'Upload or embed with a link.',
  iconX: Ix.image,
  aliases: 'image picture photo media',
  placeholder: 'Add a caption…',
  textStyle: captionStyle,
  decorate: _decorate,
  handleTop: _top,
  atomic: true,
  acceptsChildren: false,
  colorable: false,
  group: 'Media',
);

double _top(BlockContext b) => b.mobile ? 6 : 4;
