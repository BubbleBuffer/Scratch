// Temporary home for the built-ins not migrated to their own add-ons yet.
import '../../blocks/blocks.dart';
import '../../features/ai/ask_ai.dart';
import '../../registry/addon_registry.dart';
import '../../registry/block_codecs.dart';
import '../../services/legacy_host.dart';

class LegacyAddon extends Addon {
  const LegacyAddon();
  @override
  String get id => 'legacy';
  @override
  List<String> get dependsOn => const [];
  @override
  void register(AddonContext ctx) {
    registerBuiltinBlocks(ctx.blockKinds);
    registerAiBlocks(ctx.blockKinds);
    registerShellCommands(ctx.commands);
    // The temporary adapter makes the old implicit clipboard rules explicit.
    // Insert-only menu entries deliberately keep their old plain fallback.
    for (final type in ctx.blockKinds.all.toList()) {
      ctx.blockKinds.register(
        type.inGroup(
          type.group,
          codecs:
              type.codecs ?? legacyBlockCodecs(type.kind) ?? genericBlockCodecs,
        ),
      );
    }
  }
}
