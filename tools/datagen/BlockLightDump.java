// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Dumps the light properties of vanilla 1.8.8's blocks from the SRG-remapped server jar
// (see AGENTS.md: reference/tools/server-srg.jar). Nothing from vanilla is copied: the
// registry is read through reflection.
//
// Run: java -cp reference/tools/server-srg.jar tools/datagen/BlockLightDump.java > block_light.txt
//
// Output, one block per line, in id order:
//   <id> <light opacity 0-255> <light value 0-15> <uses neighbor brightness 0|1>

import java.lang.reflect.Method;
import java.util.Map;
import java.util.TreeMap;

public class BlockLightDump {
    public static void main(String[] args) throws Exception {
        Class.forName("net.minecraft.init.Bootstrap").getMethod("func_151354_b").invoke(null);
        Class<?> block = Class.forName("net.minecraft.block.Block");
        Iterable<?> registry = (Iterable<?>) block.getField("field_149771_c").get(null);
        Method idOf = block.getMethod("func_149682_b", block);
        Method opacity = block.getMethod("func_149717_k");
        Method light = block.getMethod("func_149750_m");
        Method neighbor = block.getMethod("func_149710_n");
        Map<Integer, String> lines = new TreeMap<>();
        for (Object b : registry) {
            int id = (Integer) idOf.invoke(null, b);
            lines.put(id, id + " " + opacity.invoke(b) + " " + light.invoke(b) + " "
                + (((Boolean) neighbor.invoke(b)) ? 1 : 0));
        }
        StringBuilder out = new StringBuilder();
        for (String line : lines.values()) out.append(line).append('\n');
        System.out.print(out);
    }
}
