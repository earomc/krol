// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Dumps the weather properties of vanilla 1.8.8's biomes from the SRG-remapped server
// jar (see AGENTS.md: reference/tools/server-srg.jar). Nothing from vanilla is copied:
// the biome list is read through reflection.
//
// Run: java -cp reference/tools/server-srg.jar tools/datagen/BiomeDump.java > biomes.txt
//
// Output, one biome per line, in id order:
//   <id> <temperature> <rainfall> <snowy 0|1> <lightning 0|1> <high humidity 0|1>
// (temperature and rainfall as the exact float bits, in hex)

import java.lang.reflect.Field;
import java.lang.reflect.Method;

public class BiomeDump {
    public static void main(String[] args) throws Exception {
        Class.forName("net.minecraft.init.Bootstrap").getMethod("func_151354_b").invoke(null);
        Class<?> biome = Class.forName("net.minecraft.world.biome.BiomeGenBase");
        Object[] list = (Object[]) biome.getMethod("func_150565_n").invoke(null);
        Field temperature = biome.getField("field_76750_F");
        Field rainfall = biome.getField("field_76751_G");
        Method snowy = biome.getMethod("func_76746_c");
        Method lightning = biome.getMethod("func_76738_d");
        Method humid = biome.getMethod("func_76736_e");
        StringBuilder out = new StringBuilder();
        for (int id = 0; id < list.length; id++) {
            Object b = list[id];
            if (b == null) continue;
            out.append(id).append(' ')
                .append(Integer.toHexString(Float.floatToIntBits(temperature.getFloat(b)))).append(' ')
                .append(Integer.toHexString(Float.floatToIntBits(rainfall.getFloat(b)))).append(' ')
                .append(((Boolean) snowy.invoke(b)) ? 1 : 0).append(' ')
                .append(((Boolean) lightning.invoke(b)) ? 1 : 0).append(' ')
                .append(((Boolean) humid.invoke(b)) ? 1 : 0).append('\n');
        }
        System.out.print(out);
    }
}
