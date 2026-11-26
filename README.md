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
- Solo hay una escritura en curso a la vez; una segunda solicitud se rechaza de forma explícita.
- La plataforma y el anti-cheat se vuelven a comprobar en el límite de escritura. Una consulta fallida bloquea el cambio.
- El éxito solo se informa después de releer el estado real. Un fallo de escritura, de diario o de verificación termina en `recovery_required`, nunca en éxito verificado.
- Deshacer restaura la instantánea exacta y la verifica por lectura.

`restore_all` sigue existiendo en la superficie de comandos del dominio y responde que todavía no está implementado. El adaptador real de escritura está implementado pero no se ha ejercitado en una VM desechable; esa validación destructiva es un paso posterior.

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

Las pruebas de escritura del dominio usan un adaptador falso en memoria. Las pruebas del adaptador real están marcadas para ejecutarse solo en una VM desechable de Windows 11.

## Licencia

Este proyecto se distribuye bajo la licencia [MIT](LICENSE).
