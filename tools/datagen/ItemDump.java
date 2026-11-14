// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Dumps per-item properties of vanilla 1.8.8 from the SRG-remapped server jar (see
// AGENTS.md: reference/tools/server-srg.jar). Nothing from vanilla is copied: the
// registry is read through reflection.
//
// Run: java -cp reference/tools/server-srg.jar tools/datagen/ItemDump.java > items.txt
//
// Output, one item per line, in id order:
//   <id> <max stack size> <max damage> <has subtypes 0|1> <container item id or 0>

import java.lang.reflect.Method;
import java.util.Map;
import java.util.TreeMap;

public class ItemDump {
    public static void main(String[] args) throws Exception {
        Class.forName("net.minecraft.init.Bootstrap").getMethod("func_151354_b").invoke(null);
        Class<?> item = Class.forName("net.minecraft.item.Item");
        Iterable<?> registry = (Iterable<?>) item.getField("field_150901_e").get(null);
        Method idOf = item.getMethod("func_150891_b", item);
        Method stackLimit = item.getMethod("func_77639_j");
        Method maxDamage = item.getMethod("func_77612_l");
        Method subtypes = item.getMethod("func_77614_k");
        Method container = item.getMethod("func_77668_q");
        Map<Integer, String> lines = new TreeMap<>();
        for (Object it : registry) {
            int id = (Integer) idOf.invoke(null, it);
            Object c = container.invoke(it);
            int cid = c == null ? 0 : (Integer) idOf.invoke(null, c);
            lines.put(id, id + " " + stackLimit.invoke(it) + " " + maxDamage.invoke(it) + " "
                + (((Boolean) subtypes.invoke(it)) ? 1 : 0) + " " + cid);
        }
        StringBuilder out = new StringBuilder();
        for (String line : lines.values()) out.append(line).append('\n');
        System.out.print(out);
    }
}
