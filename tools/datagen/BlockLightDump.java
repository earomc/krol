// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Dumps the light, random-tick, cube and explosion properties of vanilla 1.8.8's blocks from the SRG-remapped server jar
// (see AGENTS.md: reference/tools/server-srg.jar). Nothing from vanilla is copied: the
// registry is read through reflection.
//
// Run: java -cp reference/tools/server-srg.jar tools/datagen/BlockLightDump.java > block_light.txt
//
// Output, one block per line, in id order:
//   <id> <light opacity 0-255> <light value 0-15> <uses neighbor brightness 0|1>
//   <ticks randomly 0|1> <isVisuallyOpaque 0|1> <isFullBlock 0|1> <isFullCube 0|1>
//   <isNormalCube 0|1> <getExplosionResistance(null) as float bits in hex>

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
        Method randomly = block.getMethod("func_149653_t");
        Method resistance = block.getMethod("func_149638_a", Class.forName("net.minecraft.entity.Entity"));
        Method[] flags = {
            block.getMethod("func_176214_u"), // isVisuallyOpaque
            block.getMethod("func_149730_j"), // isFullBlock
            block.getMethod("func_149686_d"), // isFullCube
            block.getMethod("func_149721_r"), // isNormalCube
        };
        Map<Integer, String> lines = new TreeMap<>();
        for (Object b : registry) {
            int id = (Integer) idOf.invoke(null, b);
            lines.put(id, id + " " + opacity.invoke(b) + " " + light.invoke(b) + " "
                + (((Boolean) neighbor.invoke(b)) ? 1 : 0) + " "
                + (((Boolean) randomly.invoke(b)) ? 1 : 0) + flagColumns(flags, b) + " "
                + Integer.toHexString(Float.floatToIntBits((Float) resistance.invoke(b, (Object) null))));
        }
        StringBuilder out = new StringBuilder();
        for (String line : lines.values()) out.append(line).append('\n');
        System.out.print(out);
    }

    private static String flagColumns(Method[] flags, Object b) throws Exception {
        StringBuilder s = new StringBuilder();
        for (Method m : flags) s.append(((Boolean) m.invoke(b)) ? " 1" : " 0");
        return s.toString();
    }
}
