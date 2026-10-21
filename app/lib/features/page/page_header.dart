// The top of a page: the cover (full editor width, 270 tall on desktop and
// 200 on mobile), the icon (64px paper tile, overlapping the cover's
// bottom by 42px), the "Add icon / Add cover / Add comment" row that shows
// on hover, then the title the editor builds.

import 'dart:io';

import 'package:flutter/widgets.dart';

import '../../blocks/attachment.dart';
import '../../state/app_state.dart';
import '../../state/pages_store.dart';
import '../../ui/block_colors.dart';
import '../../ui/emoji_picker.dart';
import '../../ui/icon_x.dart';
import '../../ui/ui.dart';
import 'cover_picker.dart';

class PageHeader extends StatefulWidget {
  const PageHeader({
    super.key,
    required this.pageId,
    required this.icon,
    required this.cover,
    required this.textLeft,
    required this.textWidth,
    required this.width,
    required this.title,
    required this.onIcon,
    required this.onCover,
    this.readOnly = false,
    this.patch,
  });
  final String pageId;
  final String? icon, cover;
  final double textLeft, textWidth;

  /// The editor area's width (the cover spans it).
  final double width;

  /// The title field (built by the editor).
  final Widget title;

  /// New icon (null removes it).
  final ValueChanged<String?> onIcon;

  /// New cover attachment (null removes it).
  final ValueChanged<String?> onCover;
  final bool readOnly;
  final SpacePatch? patch;

  static double coverHeight(bool mobile) => mobile ? 200 : 270;

  @override
  State<PageHeader> createState() => _PageHeaderState();
}

class _PageHeaderState extends State<PageHeader> {
  bool _hover = false;

  Future<void> _pickIcon(BuildContext anchor) async {
    final e = await showEmojiPicker(
      anchor,
      anchor: context.mobile ? null : globalRectOf(anchor),
      removable: widget.icon != null,
    );
    if (e == null) return;
    widget.onIcon(e.isEmpty ? null : e);
  }

  Future<void> _pickCover(BuildContext anchor) async {
    final r = await showCoverPicker(
      anchor,
      pageId: widget.pageId,
      anchor: context.mobile ? null : globalRectOf(anchor),
      hasCover: widget.cover != null,
    );
    if (r == null) return;
    widget.onCover(r.isEmpty ? null : r);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final mobile = context.mobile;
    final icon = widget.icon;
    final cover = widget.cover;
    const iconSize = 64.0;
    final overlap = mobile ? 36.0 : 42.0;
    final showRow =
        !widget.readOnly &&
        (mobile || _hover) &&
        (icon == null || cover == null);

    final iconWidget = icon == null
        ? null
        : Builder(
            builder: (anchor) => Hover(
              key: const ValueKey('page-icon'),
              borderRadius: widget.patch == null ? Radii.panel : Radii.ctl,
              onTap: widget.readOnly ? null : () => _pickIcon(anchor),
              child: Container(
                decoration: BoxDecoration(
                  color: widget.patch?.fill ?? c.bgPrimary,
                  border: widget.patch == null
                      ? Border.all(color: c.border)
                      : null,
                  borderRadius: widget.patch == null ? Radii.panel : Radii.ctl,
                  boxShadow: c.seam,
                ),
                width: iconSize,
                height: iconSize,
                child: Center(
                  child: Text(
                    icon,
                    style: TextStyle(
                      fontSize: 40,
                      height: 1.0,
                      fontFamilyFallback: emojiFallback,
                    ),
                  ),
                ),
              ),
            ),
          );

    Widget rowButton(
      String key,
      Ix ix,
      String label,
      void Function(BuildContext) onTap,
    ) => Builder(
      builder: (anchor) => Hover(
        key: ValueKey(key),
        radius: Radii.md,
        onTap: () => onTap(anchor),
        padding: const EdgeInsets.symmetric(horizontal: 6),
        child: SizedBox(
          height: 28,
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              IconX(ix, size: 16, color: c.iconTertiary),
              const SizedBox(width: 6),
              Text(
                label,
                style: ts(context.type.ui, c.textTertiary, weight: 400),
              ),
            ],
          ),
        ),
      ),
    );

    final row = AnimatedOpacity(
      opacity: showRow ? 1 : 0,
      duration: Motion.of(context).fast,
      child: IgnorePointer(
        ignoring: !showRow,
        child: SizedBox(
          height: 30,
          child: Row(
            children: [
              if (icon == null)
                rowButton('add-icon', Ix.smile, 'Add icon', _pickIcon),
              if (cover == null)
                rowButton('add-cover', Ix.image, 'Add cover', _pickCover),
              if (!mobile)
                rowButton('add-comment', Ix.mention, 'Add comment', (_) {}),
            ],
          ),
        ),
      ),
    );

    final column = <Widget>[];
    if (cover != null) {
      final coverWidget = _Cover(
        key: const ValueKey('page-cover'),
        pageId: widget.pageId,
        cover: cover,
        height: PageHeader.coverHeight(mobile),
        textRight: widget.textLeft + widget.textWidth,
        width: widget.width,
        readOnly: widget.readOnly,
        onChange: _pickCover,
        onRemove: () => widget.onCover(null),
      );
      if (iconWidget == null) {
        column.add(coverWidget);
      } else {
        // The icon overlaps the cover's bottom edge; both sit in one Stack
        // sized to include the icon, so all of it takes taps.
        final h = PageHeader.coverHeight(mobile);
        column.add(
          SizedBox(
            height: h + iconSize - overlap,
            child: Stack(
              children: [
                Positioned(
                  left: 0,
                  right: 0,
                  top: 0,
                  height: h,
                  child: coverWidget,
                ),
                Positioned(
                  left: widget.textLeft,
                  top: h - overlap,
                  child: iconWidget,
                ),
              ],
            ),
          ),
        );
      }
    }
    final text = <Widget>[];
    if (iconWidget != null) {
      if (cover != null) {
        // Placed with the cover above.
      } else {
        text.add(SizedBox(height: mobile ? 16 : 72));
        text.add(Align(alignment: Alignment.centerLeft, child: iconWidget));
      }
      text.add(const SizedBox(height: 4));
    } else {
      text.add(SizedBox(height: cover != null ? 12 : (mobile ? 12 : 64)));
    }
    if (!widget.readOnly) text.add(row);
    text.add(widget.title);

    column.add(
      Padding(
        padding: EdgeInsets.only(left: widget.textLeft),
        child: SizedBox(
          width: widget.textWidth,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            mainAxisSize: MainAxisSize.min,
            children: text,
          ),
        ),
      ),
    );

    return MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: column,
      ),
    );
  }
}

class _Cover extends StatefulWidget {
  const _Cover({
    super.key,
    required this.pageId,
    required this.cover,
    required this.height,
    required this.textRight,
    required this.width,
    required this.readOnly,
    required this.onChange,
    required this.onRemove,
  });
  final String pageId, cover;
  final double height, textRight, width;
  final bool readOnly;
  final void Function(BuildContext anchor) onChange;
  final VoidCallback onRemove;

  @override
  State<_Cover> createState() => _CoverState();
}

class _CoverState extends State<_Cover> {
  bool _hover = false;
  bool _repositioning = false;
  double? _dragY;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final pages = context.watch<PagesStore>();
    final y = _dragY ?? pages.prefs(widget.pageId).coverY;
    final image = AttachmentBuilder(
      id: widget.cover,
      builder: (context, a, loading) {
        if (a != null && a.path.isNotEmpty) {
          return Image.file(
            File(a.path),
            fit: BoxFit.cover,
            alignment: Alignment(0, y * 2 - 1),
            width: double.infinity,
            height: widget.height,
            errorBuilder: (_, _, _) => ColoredBox(color: c.bgSecondary),
          );
        }
        return ColoredBox(color: c.bgSecondary);
      },
    );
    Widget btn(String key, String label, VoidCallback onTap) => Hover(
      key: ValueKey(key),
      onTap: onTap,
      radius: 0,
      padding: const EdgeInsets.symmetric(horizontal: 8),
      child: SizedBox(
        height: 26,
        child: Center(
          child: Text(label, style: tsRaw(12, 16, 400, c.textSecondary)),
        ),
      ),
    );
    // Desktop: on hover. Mobile: a tap on the cover shows (and hides) them.
    final show = !widget.readOnly && (_hover || _repositioning);
    return MouseRegion(
      onEnter: context.mobile ? null : (_) => setState(() => _hover = true),
      onExit: context.mobile ? null : (_) => setState(() => _hover = false),
      child: SizedBox(
        height: widget.height,
        child: Stack(
          fit: StackFit.expand,
          children: [
            if (_repositioning)
              GestureDetector(
                onVerticalDragUpdate: (d) => setState(() {
                  _dragY = ((_dragY ?? y) - d.delta.dy / widget.height).clamp(
                    0.0,
                    1.0,
                  );
                }),
                child: MouseRegion(
                  cursor: SystemMouseCursors.move,
                  child: image,
                ),
              )
            else if (context.mobile)
              GestureDetector(
                key: const ValueKey('cover-tap'),
                behavior: HitTestBehavior.opaque,
                onTap: () => setState(() => _hover = !_hover),
                child: image,
              )
            else
              image,
            if (_repositioning)
              IgnorePointer(
                child: Center(
                  child: Container(
                    padding: const EdgeInsets.symmetric(
                      horizontal: 12,
                      vertical: 6,
                    ),
                    decoration: BoxDecoration(
                      color: const Color(0x99000000),
                      borderRadius: BorderRadius.circular(Radii.sm),
                    ),
                    child: Text(
                      'Drag image to reposition',
                      style: tsRaw(14, 20, 500, const Color(0xFFFFFFFF)),
                    ),
                  ),
                ),
              ),
            if (show)
              Positioned(
                right: context.mobile
                    ? 12
                    : (widget.width - widget.textRight).clamp(
                        12.0,
                        double.infinity,
                      ),
                bottom: 10,
                child: Container(
                  decoration: BoxDecoration(
                    color: c.bgElevated,
                    borderRadius: BorderRadius.circular(Radii.sm),
                    boxShadow: c.shadowSmall,
                  ),
                  clipBehavior: Clip.antiAlias,
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: _repositioning
                        ? [
                            btn('cover-save', 'Save position', () {
                              final v = _dragY;
                              if (v != null) {
                                pages.updatePrefs(
                                  widget.pageId,
                                  (p) => p.coverY = v,
                                );
                              }
                              setState(() {
                                _repositioning = false;
                                _dragY = null;
                              });
                            }),
                            Container(width: 1, height: 26, color: c.border),
                            btn(
                              'cover-cancel',
                              'Cancel',
                              () => setState(() {
                                _repositioning = false;
                                _dragY = null;
                              }),
                            ),
                          ]
                        : [
                            Builder(
                              builder: (anchor) => btn(
                                'cover-change',
                                'Change cover',
                                () => widget.onChange(anchor),
                              ),
                            ),
                            Container(width: 1, height: 26, color: c.border),
                            btn(
                              'cover-reposition',
                              'Reposition',
                              () => setState(() => _repositioning = true),
                            ),
                            Container(width: 1, height: 26, color: c.border),
                            btn('cover-remove', 'Remove', widget.onRemove),
                          ],
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}
