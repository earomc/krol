// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Dumps vanilla 1.8.8's crafting and smelting recipes as data, in registration order,
// from the SRG-remapped server jar (see AGENTS.md: reference/tools/server-srg.jar).
// Nothing from vanilla is copied: the classes are loaded and read through reflection.
//
// Run: java -cp reference/tools/server-srg.jar tools/datagen/RecipeDump.java > recipes.txt
//
// Output, one recipe per line:
//   shaped <w> <h> <out id> <count> <damage> <in id:damage or -> ...
//   shapeless <out id> <count> <damage> <in id:damage> ...
//   special <class>
//   smelt <in id:damage> <out id> <count> <damage> <xp>

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.List;
import java.util.Map;

public class RecipeDump {
    static Method getItem;
    static Method getIdFromItem;

    static Object get(Object o, String field) throws Exception {
        Field f = o.getClass().getDeclaredField(field);
        f.setAccessible(true);
        return f.get(o);
    }

    static Object getIn(Class<?> c, Object o, String field) throws Exception {
        Field f = c.getDeclaredField(field);
        f.setAccessible(true);
        return f.get(o);
    }

    static String stack(Object s, boolean withCount) throws Exception {
        if (s == null) return "-";
        Class<?> c = s.getClass();
        Object item = getItem.invoke(s);
        int id = (Integer) getIdFromItem.invoke(null, item);
        int count = (Integer) getIn(c, s, "field_77994_a");
        int damage = (Integer) getIn(c, s, "field_77991_e");
        Object tag = getIn(c, s, "field_77990_d");
        String nbt = tag == null ? "" : " nbt";
        return withCount ? id + " " + count + " " + damage + nbt : id + ":" + damage;
    }

    public static void main(String[] args) throws Exception {
        Class.forName("net.minecraft.init.Bootstrap").getMethod("func_151354_b").invoke(null);
        Class<?> itemStack = Class.forName("net.minecraft.item.ItemStack");
        Class<?> item = Class.forName("net.minecraft.item.Item");
        getItem = itemStack.getMethod("func_77973_b");
        getIdFromItem = item.getMethod("func_150891_b", item);

        Class<?> manager = Class.forName("net.minecraft.item.crafting.CraftingManager");
        Object instance = manager.getMethod("func_77594_a").invoke(null);
        List<?> recipes = (List<?>) manager.getMethod("func_77592_b").invoke(instance);
        StringBuilder out = new StringBuilder();
        for (Object r : recipes) {
            String name = r.getClass().getSimpleName();
            if (name.equals("ShapedRecipes")) {
                int w = (Integer) get(r, "field_77576_b");
                int h = (Integer) get(r, "field_77577_c");
                Object[] items = (Object[]) get(r, "field_77574_d");
                out.append("shaped ").append(w).append(' ').append(h).append(' ')
                    .append(stack(get(r, "field_77575_e"), true));
                for (Object s : items) out.append(' ').append(stack(s, false));
            } else if (name.equals("ShapelessRecipes")) {
                out.append("shapeless ").append(stack(get(r, "field_77580_a"), true));
                for (Object s : (List<?>) get(r, "field_77579_b")) out.append(' ').append(stack(s, false));
            } else {
                out.append("special ").append(name);
            }
            out.append('\n');
        }

        Class<?> furnace = Class.forName("net.minecraft.item.crafting.FurnaceRecipes");
        Object smelting = furnace.getMethod("func_77602_a").invoke(null);
        Map<?, ?> list = (Map<?, ?>) getIn(furnace, smelting, "field_77604_b");
        Map<?, ?> xp = (Map<?, ?>) getIn(furnace, smelting, "field_77605_c");
        for (Map.Entry<?, ?> e : list.entrySet()) {
            out.append("smelt ").append(stack(e.getKey(), false)).append(' ')
                .append(stack(e.getValue(), true)).append(' ').append(xp.get(e.getValue())).append('\n');
        }
        System.out.print(out);
    }
}
