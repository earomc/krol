// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Dumps how vanilla 1.8.8 normalizes block metadata (Block.getMetaFromState of
// Block.getStateFromMeta) from the SRG-remapped server jar (see AGENTS.md:
// reference/tools/server-srg.jar). Nothing from vanilla is copied: the registry is read
// through reflection.
//
// Run: java -cp reference/tools/server-srg.jar tools/datagen/BlockMetaDump.java > block_meta.txt
//
// Output, one block per line, in id order: <id> followed by 16 values, the stored meta
// for metas 0-15 (x where getStateFromMeta throws). Blocks whose metadata all maps to
// itself are left out.

import java.lang.reflect.Method;
import java.util.Map;
import java.util.TreeMap;

public class BlockMetaDump {
    public static void main(String[] args) throws Exception {
        Class.forName("net.minecraft.init.Bootstrap").getMethod("func_151354_b").invoke(null);
        Class<?> block = Class.forName("net.minecraft.block.Block");
        Class<?> state = Class.forName("net.minecraft.block.state.IBlockState");
        Iterable<?> registry = (Iterable<?>) block.getField("field_149771_c").get(null);
        Method idOf = block.getMethod("func_149682_b", block);
        Method fromMeta = block.getMethod("func_176203_a", int.class);
        Method toMeta = block.getMethod("func_176201_c", state);
        Map<Integer, String> lines = new TreeMap<>();
        for (Object b : registry) {
            int id = (Integer) idOf.invoke(null, b);
            StringBuilder line = new StringBuilder().append(id);
            boolean identity = true;
            for (int meta = 0; meta < 16; meta++) {
                String v;
                try {
                    int m = (Integer) toMeta.invoke(b, fromMeta.invoke(b, meta));
                    v = Integer.toString(m);
                    if (m != meta) identity = false;
                } catch (Exception e) {
                    v = "x";
                    identity = false;
                }
                line.append(' ').append(v);
            }
            if (!identity) lines.put(id, line.toString());
        }
        StringBuilder out = new StringBuilder();
        for (String line : lines.values()) out.append(line).append('\n');
        System.out.print(out);
    }
}
