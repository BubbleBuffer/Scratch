// Pickers that fill a media block or a link: a file for images and files
// (from the system's file dialog, or a typed or pasted path, then copied
// into the vault's attachments), a URL for bookmarks,
// and a page search for "Link to page" and mentions. Popovers on desktop,
// sheets on mobile.

import '../core/debounce.dart';

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../core/file_dialogs.dart';
import '../core/vault.dart';
import '../shell/page_stack.dart';
import '../state/app_state.dart';
import '../ui/ui.dart';

Future<T?> _show<T>(BuildContext context, Rect? anchor, Widget Function(BuildContext ctx) body, {double width = 420}) {
  if (context.mobile || anchor == null) {
    return showSheet<T>(context, (ctx) => SheetSurface(children: [body(ctx)]));
  }
  return showPopover<T>(context, anchor, (ctx) => MenuSurface(width: width, child: body(ctx)));
}

/// Adds a file picked in the system dialog to the vault's attachments and
/// returns the attachment id.
Future<String> addPickedAttachment(Vault vault, PickedFile f) async {
  final path = f.path;
  if (path != null) return vault.run((v) => v.addAttachmentFromPath(path: path), writes: true);
  final bytes = await f.bytes();
  return vault.run((v) => v.addAttachmentBytes(name: f.name, bytes: bytes), writes: true);
}

/// Asks for a file (system dialog, or a typed path) and adds it to the
/// vault's attachments. Completes with the attachment id.
Future<String?> pickAttachment(BuildContext context, Rect? anchor, {required bool image}) {
  final vault = context.app.vault;
  final dialogs = FileDialogs.instance;
  return _show<String>(
    context,
    anchor,
    (ctx) => _FieldPicker(
      key: const ValueKey('attachment-picker'),
      tab: 'Upload',
      placeholder: dialogs.available
          ? 'Or paste a file path…'
          : (image ? 'Path to an image file…' : 'Path to a file…'),
      button: image ? 'Upload image' : 'Upload file',
      hint: 'Copies the file into this vault.',
      onChoose: !dialogs.available
          ? null
          : () async {
              final files = await dialogs.pickFiles(title: image ? 'Upload image' : 'Upload file', images: image);
              if (files.isEmpty) return null;
              try {
                final id = await addPickedAttachment(vault, files.first);
                if (ctx.mounted) Navigator.of(ctx).pop(id);
                return null;
              } catch (e) {
                return 'Could not add the file.';
              }
            },
      onSubmit: (path) async {
        final p = path.trim().replaceAll('"', '');
        if (p.isEmpty) return 'Enter a file path.';
        try {
          final id = await vault.run((v) => v.addAttachmentFromPath(path: p), writes: true);
          if (ctx.mounted) Navigator.of(ctx).pop(id);
          return null;
        } catch (e) {
          return 'Could not add the file.';
        }
      },
    ),
  );
}

/// Asks for a web address (bookmarks).
Future<String?> pickUrl(BuildContext context, Rect? anchor) => _show<String>(
      context,
      anchor,
      (ctx) => _FieldPicker(
        key: const ValueKey('url-picker'),
        tab: 'Web bookmark',
        placeholder: 'Paste in https://…',
        button: 'Create bookmark',
        hint: 'Create a visual bookmark from a link.',
        onSubmit: (v) async {
          final u = v.trim();
          if (u.isEmpty) return 'Enter a link.';
          Navigator.of(ctx).pop(u.contains('://') ? u : 'https://$u');
          return null;
        },
      ),
    );

class _FieldPicker extends StatefulWidget {
  const _FieldPicker({
    super.key,
    required this.tab,
    required this.placeholder,
    required this.button,
    required this.hint,
    required this.onSubmit,
    this.onChoose,
  });
  final String tab, placeholder, button, hint;

  /// Returns an error to show, or null when done.
  final Future<String?> Function(String value) onSubmit;

  /// Opens the system file dialog (the button does this when set; the
  /// field still takes a typed path). Returns an error, or null.
  final Future<String?> Function()? onChoose;

  @override
  State<_FieldPicker> createState() => _FieldPickerState();
}

class _FieldPickerState extends State<_FieldPicker> {
  final _ctl = TextEditingController();
  final _focus = FocusNode();
  String? _error;
  bool _busy = false;

  @override
  void dispose() {
    _ctl.dispose();
    _focus.dispose();
    super.dispose();
  }

  Future<void> _submit() => _do(() => widget.onSubmit(_ctl.text));

  Future<void> _do(Future<String?> Function() f) async {
    if (_busy) return;
    setState(() => _busy = true);
    final e = await f();
    if (mounted) {
      setState(() {
        _busy = false;
        _error = e;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final choose = widget.onChoose;
    final field = Container(
      height: 32,
      padding: const EdgeInsets.symmetric(horizontal: 8),
      alignment: Alignment.centerLeft,
      decoration: BoxDecoration(
        color: c.bgHover,
        borderRadius: BorderRadius.circular(Radii.md),
        border: Border.all(color: c.border),
      ),
      child: PlainField(
        key: const ValueKey('picker-field'),
        controller: _ctl,
        focusNode: _focus,
        autofocus: choose == null,
        style: ts(ty.ui, c.textPrimary, weight: 400),
        placeholder: widget.placeholder,
        textInputAction: TextInputAction.done,
        onSubmitted: (_) => _submit(),
      ),
    );
    final button = Center(
      child: SizedBox(
        width: 260,
        child: PrimaryButton(
          key: const ValueKey('picker-submit'),
          label: widget.button,
          onTap: choose != null ? () => _do(choose) : _submit,
        ),
      ),
    );
    // With a file dialog the button opens it and the field is the fallback.
    final (first, second) = choose != null ? (button, field) : (field, button);
    return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Container(
        height: 40,
        padding: const EdgeInsets.symmetric(horizontal: 8),
        decoration: BoxDecoration(border: Border(bottom: BorderSide(color: c.border))),
        child: Row(children: [
          Container(
            height: 40,
            padding: const EdgeInsets.symmetric(horizontal: 6),
            alignment: Alignment.center,
            decoration: BoxDecoration(border: Border(bottom: BorderSide(color: c.textPrimary, width: 2))),
            child: Text(widget.tab, style: ts(ty.ui, c.textPrimary)),
          ),
        ]),
      ),
      Padding(padding: const EdgeInsets.fromLTRB(12, 12, 12, 0), child: first),
      Padding(padding: const EdgeInsets.fromLTRB(12, 10, 12, 0), child: second),
      Padding(
        padding: const EdgeInsets.fromLTRB(12, 8, 12, 12),
        child: Text(_error ?? widget.hint,
            textAlign: TextAlign.center, style: ts(ty.uiSmall, _error != null ? c.inlineCodeText : c.textTertiary, weight: 400)),
      ),
    ]);
  }
}

// ---------------------------------------------------------------------------
// Page search.

/// A page found by [searchPages].
class PageHit {
  const PageHit(this.id, this.title, this.icon);
  final String id, title;
  final String? icon;
}

/// Pages matching [query] (recent pages when it is empty).
Future<List<PageHit>> searchPages(BuildContext context, String query, {int limit = 8}) async {
  final app = context.app;
  final q = query.trim();
  if (q.isEmpty) {
    final rows = await app.vault.run((v) => v.listPages(offset: 0, limit: limit));
    return [for (final r in rows) PageHit(r.id, r.title, r.icon)];
  }
  final hits = await app.vault.run((v) => v.search(query: q, limit: limit * 2));
  final out = <PageHit>[];
  final seen = <String>{};
  for (final h in hits) {
    if (!seen.add(h.id)) continue;
    final meta = app.pages.meta(h.id);
    if (meta != null && meta.inTrash) continue;
    out.add(PageHit(h.id, h.title, meta?.icon));
    if (out.length >= limit) break;
  }
  return out;
}

/// Asks for a page ("Link to page"). Completes with its id.
Future<String?> pickPage(BuildContext context, Rect? anchor, {String? exclude}) => _show<String>(
      context,
      anchor,
      (ctx) => PageSearchList(
        key: const ValueKey('page-picker'),
        exclude: exclude,
        onPick: (id) => Navigator.of(ctx).pop(id),
      ),
      width: 320,
    );

class PageSearchList extends StatefulWidget {
  const PageSearchList({super.key, required this.onPick, this.exclude});
  final ValueChanged<String> onPick;
  final String? exclude;

  @override
  State<PageSearchList> createState() => _PageSearchListState();
}

class _PageSearchListState extends State<PageSearchList> {
  final _ctl = TextEditingController();
  final _focus = FocusNode();
  List<PageHit> _hits = const [];
  Timer? _debounce;
  int _gen = 0;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _search());
  }

  @override
  void dispose() {
    _debounce?.cancel();
    _ctl.dispose();
    _focus.dispose();
    super.dispose();
  }

  Future<void> _search() async {
    final g = ++_gen;
    final hits = await searchPages(context, _ctl.text);
    if (!mounted || g != _gen) return;
    setState(() => _hits = [for (final h in hits) if (h.id != widget.exclude) h]);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    return Padding(
      padding: const EdgeInsets.all(6),
      child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Container(
          height: 30,
          margin: const EdgeInsets.fromLTRB(2, 2, 2, 6),
          padding: const EdgeInsets.symmetric(horizontal: 8),
          alignment: Alignment.centerLeft,
          decoration: BoxDecoration(
            color: c.bgHover,
            borderRadius: BorderRadius.circular(Radii.md),
            border: Border.all(color: c.border),
          ),
          child: PlainField(
            key: const ValueKey('page-picker-field'),
            controller: _ctl,
            focusNode: _focus,
            autofocus: true,
            style: ts(ty.ui, c.textPrimary, weight: 400),
            placeholder: 'Search for a page…',
            onChanged: (_) {
              _debounce?.cancel();
              _debounce = Timer(Debounce.testDelay(const Duration(milliseconds: 150)), _search);
            },
            onSubmitted: (_) {
              if (_hits.isNotEmpty) widget.onPick(_hits.first.id);
            },
          ),
        ),
        MenuHeader(_ctl.text.trim().isEmpty ? 'Recent pages' : (_hits.isEmpty ? 'No results' : 'Pages')),
        for (final h in _hits) _row(h),
      ]),
    );
  }

  /// A hit's row: an untitled page shows its first line in tertiary italics.
  MenuItem _row(PageHit h) {
    final label = pageLabelOf(context, h.id, h.title);
    return MenuItem(
      key: ValueKey('page-pick-${h.id}'),
      label: label.text,
      firstLine: label.firstLine,
      leading: PageIcon(icon: h.icon, size: context.mobile ? 22 : 20),
      onTap: () => widget.onPick(h.id),
    );
  }
}
