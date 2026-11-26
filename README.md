# Optimi-ez

Optimizador de Windows para juegos, escrito en Rust.

> **Estado: primera escritura verificada.** La aplicación puede aplicar una única acción de servicio aprobada (DiagTrack como decisión de privacidad), comprobarla por lectura y deshacerla. El adaptador real de escritura todavía no se ha ejercitado en una VM desechable; no la uses como optimizador general ni para cambiar servicios en tu equipo.

## Objetivo

Crear perfiles por juego que identifiquen los servicios de Windows con evidencia de que afectan negativamente la experiencia gamer y permitan deshabilitarlos de forma transparente, segura y reversible. Que un servicio esté activo no significa por sí solo que perjudique el rendimiento: cada recomendación deberá explicar su evidencia y validarse con mediciones.

## Principios de seguridad

- Mostrar cada cambio propuesto, sus posibles efectos y riesgos; pedir confirmación explícita antes de aplicarlo.
- No desactivar servicios críticos ni protecciones de seguridad.
- Guardar el estado original y ofrecer una forma comprobable de restaurarlo antes de habilitar cambios.
- No hacer cambios ocultos ni automáticos en segundo plano.
- Evitar promesas de mejora sin mediciones reproducibles.

## Estado

La aplicación Tauri 2 lee el estado real del servicio DiagTrack, el plan de energía activo, la versión de Windows y la cuenta interactiva. La vista previa es de solo lectura e indica cuándo un valor no está disponible.

La primera escritura verificada ya está implementada en el dominio:

- Antes de cualquier efecto externo se guarda una instantánea exacta del servicio (existencia, tipo de inicio y estado de ejecución) y una entrada de diario duradera.
- Aplicar exige un consentimiento ligado a una vista previa vigente; una vista previa obsoleta o un estado que cambió se rechazan.
- Solo hay una escritura en curso a la vez, incluso entre procesos distintos: un bloqueo exclusivo del sistema operativo cubre la reserva del identificador, el efecto y el registro en el diario.
- La plataforma y el anti-cheat se vuelven a comprobar en el límite de escritura. Una consulta fallida bloquea el cambio.
- El éxito solo se informa después de releer el estado real. Un fallo de escritura, de diario o de verificación termina en `recovery_required`, nunca en éxito verificado.
- La instantánea conserva el tipo de inicio retrasado de los servicios automáticos y lo restaura con la misma API de configuración.
- Deshacer relee el estado actual antes de escribir: si ya coincide con la instantánea solo verifica; solo escribe cuando el estado completo es exactamente la instantánea, el objetivo o el último estado observado de esa transacción. Cualquier otra combinación se rechaza como conflicto y nunca se sobrescribe.
- Si la compilación de Windows cambió desde que se registró la transacción, deshacer no escribe: devuelve el diff entre la instantánea y el estado actual y entre la compilación registrada y la actual, y exige una confirmación explícita ligada a la compilación vigente.
- Mientras exista una transacción en recuperación no se puede deshacer otra transacción ya verificada.

El panel de recuperación expone las transacciones pendientes del diario, con el servicio y la instantánea a la que volverían, y permite restaurarlas una a una incluso después de reiniciar. Las escrituras nuevas siguen bloqueadas hasta que no queda ninguna pendiente. Una transacción que ya no se puede restaurar, por ejemplo porque el servicio desapareció, se puede abandonar de forma explícita: la interfaz pide confirmación con una advertencia y el diario conserva la decisión sin restaurar nada. `restore_all` aplica la misma ruta a todas las pendientes, enumera las que no puede restaurar en lugar de detenerse en la primera y devuelve las demás restauradas.

La aplicación no promete FPS, latencia ni mejoras porcentuales. No la uses todavía para cambiar servicios en un equipo que no sea una VM de prueba.

## Pruebas

La interfaz usa el runner integrado de Node.js y no tiene dependencias npm:

```bash
npm test
```

El comando ejecuta `ui/main.test.mjs` con `node --test`.

El dominio y el adaptador se prueban con Cargo:

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

Las pruebas de escritura del dominio usan un adaptador falso en memoria. Las pruebas del adaptador real están marcadas para ejecutarse solo en una VM desechable de Windows 11. La prueba de escritura aplica y deshace DiagTrack, exige una shell elevada y no corre sin el cerrojo explícito:

```bash
OPTIMI_VM_WRITE_TEST=1 cargo test -p optimi-windows --test windows_write -- --ignored
```

## Arnés de VM

El arnés en PowerShell para Windows restaura y verifica `after-ac6` antes y después de la prueba, compila el binario con `-C target-feature=+crt-static`, registra el build, la identidad del guest, el snapshot, la salida literal y el código de salida en `C:/Users/david/VMs/runs/`. El comando de uso, desde la raíz del repo, es:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/run-vm-harness.ps1
```

`-h` muestra la ayuda y `--dry-run` imprime el plan sin acceder a la VM ni escribir archivos. Se puede elegir otro snapshot con `--snapshot <nombre>` o el test `windows_write` con `--test windows_write`; este último solo corre si el proceso del guest tiene integridad alta, y el arnés lo rechaza si no la detecta.

## Licencia

Este proyecto se distribuye bajo la licencia [MIT](LICENSE).
