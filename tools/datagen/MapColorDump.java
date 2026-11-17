// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Dumps the map color of every block state of vanilla 1.8.8 (Block.getMapColor of
// Block.getStateFromMeta) from the SRG-remapped server jar (see AGENTS.md:
// reference/tools/server-srg.jar). Nothing from vanilla is copied: the registry is read
// through reflection.
//
// Run: java -cp reference/tools/server-srg.jar tools/datagen/MapColorDump.java > map_colors.txt
//
// Output, one block per line, in id order: <id> followed by 16 values, the map color
// index for metas 0-15 (the color of meta 0 where getStateFromMeta throws).

import java.lang.reflect.Method;
import java.util.Map;
import java.util.TreeMap;

public class MapColorDump {
    public static void main(String[] args) throws Exception {
        Class.forName("net.minecraft.init.Bootstrap").getMethod("func_151354_b").invoke(null);
        Class<?> block = Class.forName("net.minecraft.block.Block");
        Class<?> state = Class.forName("net.minecraft.block.state.IBlockState");
        Class<?> mapColor = Class.forName("net.minecraft.block.material.MapColor");
        Iterable<?> registry = (Iterable<?>) block.getField("field_149771_c").get(null);
        Method idOf = block.getMethod("func_149682_b", block);
        Method fromMeta = block.getMethod("func_176203_a", int.class);
        Method colorOf = block.getMethod("func_180659_g", state);
        java.lang.reflect.Field index = mapColor.getField("field_76290_q");
        Map<Integer, String> lines = new TreeMap<>();
        for (Object b : registry) {
            int id = (Integer) idOf.invoke(null, b);
            StringBuilder line = new StringBuilder().append(id);
            int first = -1;
            for (int meta = 0; meta < 16; meta++) {
                int v;
                try {
                    v = index.getInt(colorOf.invoke(b, fromMeta.invoke(b, meta)));
                } catch (Exception e) {
                    v = first;
                }
                if (meta == 0) first = v;
                line.append(' ').append(v);
            }
            lines.put(id, line.toString());
        }
        StringBuilder out = new StringBuilder();
        for (String line : lines.values()) out.append(line).append('\n');
        System.out.print(out);
    }
}
