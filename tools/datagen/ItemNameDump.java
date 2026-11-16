// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Dumps the en_US display names the vanilla 1.8.8 server gives item stacks
// (ItemStack.getDisplayName without a custom name), from the SRG-remapped server jar
// (see AGENTS.md: reference/tools/server-srg.jar). Nothing from vanilla is copied: the
// registry is read through reflection and the server translates with its own lang file.
//
// Run: java -cp reference/tools/server-srg.jar tools/datagen/ItemNameDump.java > item_names.txt
//
// Output, tab-separated, in id order: <id> <damage> <name>. Damage 0 is listed for every
// item; items with subtypes also list damages 1-255 whose name differs from damage 0's.
// Names that depend on NBT (skull owners, book titles) or on damage values above 255
// (potions) are not covered.

import java.lang.reflect.Constructor;
import java.lang.reflect.Method;
import java.util.Map;
import java.util.TreeMap;

public class ItemNameDump {
    public static void main(String[] args) throws Exception {
        Class.forName("net.minecraft.init.Bootstrap").getMethod("func_151354_b").invoke(null);
        Class<?> item = Class.forName("net.minecraft.item.Item");
        Class<?> stack = Class.forName("net.minecraft.item.ItemStack");
        Iterable<?> registry = (Iterable<?>) item.getField("field_150901_e").get(null);
        Method idOf = item.getMethod("func_150891_b", item);
        Method subtypes = item.getMethod("func_77614_k");
        Method displayName = stack.getMethod("func_82833_r");
        Constructor<?> make = stack.getConstructor(item, int.class, int.class);
        Map<Integer, StringBuilder> lines = new TreeMap<>();
        for (Object it : registry) {
            int id = (Integer) idOf.invoke(null, it);
            StringBuilder out = new StringBuilder();
            String base = name(displayName, make, it, 0);
            out.append(id).append('\t').append(0).append('\t').append(base).append('\n');
            if ((Boolean) subtypes.invoke(it)) {
                for (int damage = 1; damage <= 255; damage++) {
                    String n = name(displayName, make, it, damage);
                    if (!n.equals(base)) {
                        out.append(id).append('\t').append(damage).append('\t').append(n).append('\n');
                    }
                }
            }
            lines.put(id, out);
        }
        StringBuilder all = new StringBuilder();
        for (StringBuilder b : lines.values()) all.append(b);
        System.out.print(all);
    }

    private static String name(Method displayName, Constructor<?> make, Object item, int damage) {
        try {
            return (String) displayName.invoke(make.newInstance(item, 1, damage));
        } catch (Exception e) {
            return "";
        }
    }
}
