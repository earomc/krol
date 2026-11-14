// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Dumps vanilla 1.8.8's foods from the SRG-remapped server jar (see AGENTS.md:
// reference/tools/server-srg.jar). Nothing from vanilla is copied: the item registry is
// read through reflection.
//
// Run: java -cp reference/tools/server-srg.jar tools/datagen/FoodDump.java > foods.txt
//
// Output, one line per food item and damage value (0-3 for items with subtypes), in id
// order:
//   <id> <damage> <hunger> <saturation float bits, hex> <always edible 0|1>
//   <has a potion effect 0|1> <leftover item id or 0>

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.Map;
import java.util.TreeMap;

public class FoodDump {
    public static void main(String[] args) throws Exception {
        Class.forName("net.minecraft.init.Bootstrap").getMethod("func_151354_b").invoke(null);
        Class<?> item = Class.forName("net.minecraft.item.Item");
        Class<?> food = Class.forName("net.minecraft.item.ItemFood");
        Class<?> stackClass = Class.forName("net.minecraft.item.ItemStack");
        Class<?> soup = Class.forName("net.minecraft.item.ItemSoup");
        Iterable<?> registry = (Iterable<?>) item.getField("field_150901_e").get(null);
        Method idOf = item.getMethod("func_150891_b", item);
        Method subtypes = item.getMethod("func_77614_k");
        Method heal = food.getMethod("func_150905_g", stackClass);
        Method saturation = food.getMethod("func_150906_h", stackClass);
        Field always = food.getDeclaredField("field_77852_bZ");
        always.setAccessible(true);
        Field potion = food.getDeclaredField("field_77851_ca");
        potion.setAccessible(true);
        Map<Integer, String> lines = new TreeMap<>();
        for (Object it : registry) {
            if (!food.isInstance(it)) continue;
            int id = (Integer) idOf.invoke(null, it);
            boolean sub = (Boolean) subtypes.invoke(it);
            StringBuilder out = new StringBuilder();
            for (int damage = 0; damage < (sub ? 4 : 1); damage++) {
                Object stack = stackClass.getConstructor(item, int.class, int.class).newInstance(it, 1, damage);
                float sat = (Float) saturation.invoke(it, stack);
                boolean special = !food.equals(it.getClass()) && !soup.isInstance(it);
                out.append(id).append(' ').append(damage).append(' ')
                    .append(heal.invoke(it, stack)).append(' ')
                    .append(Integer.toHexString(Float.floatToIntBits(sat))).append(' ')
                    .append(always.getBoolean(it) ? 1 : 0).append(' ')
                    .append(potion.getInt(it) > 0 || special && id != 391 && id != 392 ? 1 : 0).append(' ')
                    .append(soup.isInstance(it) ? 281 : 0).append('\n');
            }
            lines.put(id, out.toString());
        }
        for (String l : lines.values()) System.out.print(l);
    }
}
