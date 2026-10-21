// Backlinks at the end of the page: "Linked from", then each page that
// links here (a link-to-page block or an @-mention) with its icon and its
// title. More than three collapse behind "N more". Hidden when nothing links
// here, or when the only link is the parent's sub-page block.

import '../../core/debounce.dart';

import 'dart:async';

import 'package:flutter/widgets.dart';

import '../../core/change_feed.dart';
import '../../core/models.dart';
import '../../editor/rich_controller.dart' show inlinePageTarget;
import '../../shell/navigation.dart';
import '../../state/app_state.dart';
import '../../ui/ui.dart';

class BacklinksBar extends StatefulWidget {
  const BacklinksBar({
    super.key,
    required this.pageId,
    this.parent,
    this.left = 0,
    this.width,
  });
  final String pageId;

  /// The page's parent: its sub-page block alone is not a backlink.
  final String? parent;

  /// The text column (the editor's geometry).
  final double left;
  final double? width;

  @override
  State<BacklinksBar> createState() => _BacklinksBarState();
}

class _BacklinksBarState extends State<BacklinksBar> {
  List<PageRow> _rows = const [];
  bool _all = false;
  ChangeFeed? _feed;
  int _gen = 0;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final f = context.app.feed;
    if (f != _feed) {
      _feed?.removeListener(_onChanges);
      _feed = f..addListener(_onChanges);
      _load();
    }
  }

  @override
  void didUpdateWidget(BacklinksBar old) {
    super.didUpdateWidget(old);
    if (old.pageId != widget.pageId || old.parent != widget.parent) _load();
  }

  @override
  void dispose() {
    _debounce?.cancel();
    _feed?.removeListener(_onChanges);
    super.dispose();
  }

  void _onChanges(ChangeBatch b) {
    // Any page may have gained or lost a link here.
    if (b.changes.isEmpty) return;
    _debounce?.cancel();
    _debounce = Timer(
      Debounce.testDelay(const Duration(milliseconds: 400)),
      _load,
    );
  }

  Timer? _debounce;

  Future<void> _load() async {
    final g = ++_gen;
    final app = context.app;
    try {
      final id = widget.pageId;
      var rows = [
        for (final r in await app.vault.run((v) => v.backlinks(id: id)))
          if (r.id != id) r,
      ];
      // A parent that links here only through the sub-page block holding
      // this page is structure, not a backlink.
      if (rows.length == 1 && rows.first.id == widget.parent) {
        final parent = await app.vault.run(
          (v) => v.loadPage(id: widget.parent!),
        );
        final mentions = parent.blocks.any(
          (b) => b.marks.any((m) => inlinePageTarget(m) == id),
        );
        if (!mentions) rows = const [];
      }
      if (!mounted || g != _gen) return;
      setState(() => _rows = rows);
    } catch (_) {}
  }

  @override
  Widget build(BuildContext context) {
    if (_rows.isEmpty) return const SizedBox.shrink();
    final c = context.colors;
    final shown = _all || _rows.length <= 3 ? _rows : _rows.sublist(0, 3);
    final list = Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Padding(
          padding: const EdgeInsets.only(left: 2, bottom: 4),
          child: Text(
            'Linked from',
            style: ts(context.type.uiSmall, c.textTertiary, weight: 500),
          ),
        ),
        for (final r in shown)
          Hover(
            key: ValueKey('backlink-${r.id}'),
            radius: Radii.sm,
            onTap: () => openPage(context, r.id, r.title, stack: true),
            padding: const EdgeInsets.symmetric(horizontal: 2),
            child: SizedBox(
              height: 26,
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  PageIcon(icon: r.icon, size: 16, color: c.iconSecondary),
                  const SizedBox(width: 6),
                  Flexible(
                    child: Container(
                      decoration: BoxDecoration(
                        border: Border(bottom: BorderSide(color: c.divider)),
                      ),
                      child: PageLabel(
                        id: r.id,
                        title: r.title,
                        style: tsRaw(14, 18, 500, c.textPrimary),
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        if (_rows.length > 3)
          TextBtn(
            _all ? 'Show less' : '${_rows.length - 3} more',
            key: const ValueKey('backlinks-more'),
            height: 24,
            color: c.textTertiary,
            onTap: () => setState(() => _all = !_all),
          ),
      ],
    );
    return Padding(
      key: const ValueKey('backlinks'),
      padding: EdgeInsets.only(left: widget.left, top: 24, bottom: 8),
      child: Align(
        alignment: Alignment.topLeft,
        child: SizedBox(
          width: widget.width,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            mainAxisSize: MainAxisSize.min,
            children: [
              HLine(color: c.divider),
              const SizedBox(height: 10),
              list,
            ],
          ),
        ),
      ),
    );
  }
}
