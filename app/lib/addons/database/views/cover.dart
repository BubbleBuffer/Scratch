// Card previews of gallery and board cards: the row page's cover, else its
// first image, else (preview "content") the first lines of its text.
// Loaded once per row version, only for the cards on screen.

import 'dart:io';

import 'package:flutter/widgets.dart';

import '../../../core/models.dart';
import '../../../core/vault.dart';
import '../../../ui/ui.dart';

class CardPreview {
  const CardPreview({this.image, this.lines = const []});

  /// A local image file (cover or first image).
  final String? image;

  /// The first text lines of the page.
  final List<String> lines;

  bool get isEmpty => image == null && lines.isEmpty;
}

class PreviewCache {
  PreviewCache._();
  static final _done = <String, CardPreview>{};
  static final _loading = <String, Future<CardPreview>>{};

  static String _key(PageRow r) => '${r.id}@${r.updatedMs}';

  /// The preview of [r] if loaded (null: start loading).
  static CardPreview? peek(PageRow r) => _done[_key(r)];

  static Future<CardPreview> load(Vault vault, PageRow r) {
    final k = _key(r);
    final have = _done[k];
    if (have != null) return Future.value(have);
    return _loading[k] ??= _fetch(vault, r).then((p) {
      _done[k] = p;
      _loading.remove(k);
      return p;
    });
  }

  static Future<CardPreview> _fetch(Vault vault, PageRow r) async {
    try {
      final p = await vault.run((v) => v.loadPage(id: r.id));
      String? att = p.cover;
      final lines = <String>[];
      for (final b in p.blocks) {
        if (att == null && b.kind == Kinds.image && b.target != null) att = b.target;
        if (b.text.isNotEmpty && lines.length < 6 && b.kind != Kinds.code) lines.add(b.text);
      }
      String? path;
      if (att != null) {
        final id = att;
        final found = await vault.run((v) => v.attachmentPath(id: id));
        path = File(found).existsSync() ? found : null;
      }
      return CardPreview(image: path, lines: lines);
    } catch (_) {
      return const CardPreview();
    }
  }
}

/// The preview area of a card: an image, the page's text, or a blank
/// (preview "content" with an empty page).
class CardCover extends StatefulWidget {
  const CardCover({super.key, required this.vault, required this.row, required this.height, required this.content});
  final Vault vault;
  final PageRow row;
  final double height;

  /// Show the page text when there is no image.
  final bool content;

  @override
  State<CardCover> createState() => _CardCoverState();
}

class _CardCoverState extends State<CardCover> {
  CardPreview? _p;

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void didUpdateWidget(CardCover old) {
    super.didUpdateWidget(old);
    if (old.row.id != widget.row.id || old.row.updatedMs != widget.row.updatedMs) _load();
  }

  void _load() {
    _p = PreviewCache.peek(widget.row);
    if (_p != null) return;
    PreviewCache.load(widget.vault, widget.row).then((p) {
      if (mounted) setState(() => _p = p);
    });
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final p = _p;
    Widget child;
    if (p?.image != null) {
      child = Image.file(File(p!.image!), fit: BoxFit.cover, width: double.infinity, height: widget.height, cacheHeight: (widget.height * 2).round());
    } else if (widget.content && p != null && p.lines.isNotEmpty) {
      child = Padding(
        padding: const EdgeInsets.fromLTRB(14, 12, 14, 0),
        child: Text(p.lines.join('\n'),
            maxLines: (widget.height / 16).floor(),
            overflow: TextOverflow.clip,
            style: tsRaw(10, 15, 400, c.textSecondary)),
      );
    } else {
      child = const SizedBox.expand();
    }
    return SizedBox(
      height: widget.height,
      width: double.infinity,
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: p?.image != null ? null : c.bgSecondary,
          border: Border(bottom: BorderSide(color: c.borderTranslucent)),
        ),
        child: ClipRect(child: child),
      ),
    );
  }
}
