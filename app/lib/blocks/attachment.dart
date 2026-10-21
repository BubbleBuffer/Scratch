// Resolving an attachment id to a local file. A missing file is fetched
// from the hub through SyncStore.ensureAttachment; until then (or when it
// cannot be fetched) the block shows a placeholder.

import 'dart:io';

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../state/app_state.dart';
import '../state/sync_store.dart';

/// A resolved attachment: its path on disk and its metadata.
class LocalAttachment {
  const LocalAttachment(this.path, this.info);
  final String path;
  final AttachmentInfo? info;
}

final _cache = <String, Future<LocalAttachment?>>{};

/// Forgets resolved attachments (tests open several vaults).
void clearAttachmentCache() => _cache.clear();

Future<LocalAttachment?> resolveAttachment(BuildContext context, String id) {
  final app = context.app;
  final sync = context.read<SyncStore>();
  return _cache.putIfAbsent(id, () async {
    AttachmentInfo? info;
    try {
      info = await app.vault.run((v) => v.attachmentInfo(id: id));
    } catch (_) {}
    final path = await sync.ensureAttachment(id);
    if (path == null || !File(path).existsSync()) {
      _cache.remove(id);
      return info == null ? null : LocalAttachment('', info);
    }
    return LocalAttachment(path, info);
  });
}

/// Builds [builder] with the attachment once it is resolved (null while
/// loading or when it is missing).
class AttachmentBuilder extends StatefulWidget {
  const AttachmentBuilder({super.key, required this.id, required this.builder});
  final String id;
  final Widget Function(BuildContext context, LocalAttachment? a, bool loading) builder;

  @override
  State<AttachmentBuilder> createState() => _AttachmentBuilderState();
}

class _AttachmentBuilderState extends State<AttachmentBuilder> {
  Future<LocalAttachment?>? _f;
  String? _id;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _load();
  }

  @override
  void didUpdateWidget(AttachmentBuilder old) {
    super.didUpdateWidget(old);
    _load();
  }

  void _load() {
    if (_id == widget.id && _f != null) return;
    _id = widget.id;
    _f = resolveAttachment(context, widget.id);
  }

  @override
  Widget build(BuildContext context) => FutureBuilder<LocalAttachment?>(
        future: _f,
        builder: (context, s) =>
            widget.builder(context, s.data, s.connectionState != ConnectionState.done),
      );
}

/// "12.4 KB".
String formatBytes(int n) {
  if (n < 1024) return '$n B';
  if (n < 1024 * 1024) return '${(n / 1024).toStringAsFixed(1)} KB';
  if (n < 1024 * 1024 * 1024) return '${(n / 1024 / 1024).toStringAsFixed(1)} MB';
  return '${(n / 1024 / 1024 / 1024).toStringAsFixed(1)} GB';
}
