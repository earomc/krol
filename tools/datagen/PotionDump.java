// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Dumps vanilla 1.8.8's status effects (Potion.potionTypes) from the SRG-remapped server
// jar (see AGENTS.md: reference/tools/server-srg.jar), read through reflection.
//
// Run: java -cp reference/tools/server-srg.jar tools/datagen/PotionDump.java > potions.txt
//
// Output, one effect per line, in id order:
//   <id> <liquid color, hex> <instant 0|1> <bad 0|1> <name> <effectiveness> <usable 0|1>

public class PotionDump {
    public static void main(String[] args) throws Exception {
        Class.forName("net.minecraft.init.Bootstrap").getMethod("func_151354_b").invoke(null);
        Class<?> potion = Class.forName("net.minecraft.potion.Potion");
        Object[] types = (Object[]) potion.getField("field_76425_a").get(null);
        java.lang.reflect.Field bad = potion.getDeclaredField("field_76418_K");
        bad.setAccessible(true);
        StringBuilder out = new StringBuilder();
        for (Object p : types) {
            if (p == null) continue;
            out.append(potion.getMethod("func_76396_c").invoke(p)).append(' ')
                .append(Integer.toHexString((Integer) potion.getMethod("func_76401_j").invoke(p))).append(' ')
                .append(((Boolean) potion.getMethod("func_76403_b").invoke(p)) ? 1 : 0).append(' ')
                .append(bad.getBoolean(p) ? 1 : 0).append(' ')
                .append(potion.getMethod("func_76393_a").invoke(p)).append(' ')
                .append(potion.getMethod("func_76388_g").invoke(p)).append(' ')
                .append(((Boolean) potion.getMethod("func_76395_i").invoke(p)) ? 1 : 0).append('\n');
        }
        System.out.print(out);
    }
}
