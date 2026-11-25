"use client";

import type { ReactNode } from "react";
import { MotionConfig } from "motion/react";
import { ShapeProvider, type ShapeVariant } from "./shape-context";
import { SizeProvider, type SizeVariant } from "./size-context";
import { IconProvider, type IconComponent, type IconName } from "./icon-context";
import { LinkProvider, type LinkComponent } from "./link-context";

export interface FluidProviderProps {
  children: ReactNode;
  /** Corner style of every component. Readable and switchable at runtime through useShapeContext(). */
  shape?: ShapeVariant;
  /** Size step of every control. Readable and switchable at runtime through useSizeContext(). */
  size?: SizeVariant;
  /** Icons by slot name. Slots left out keep Lucide. */
  icons?: Partial<Record<IconName, IconComponent>>;
  /** Optional router-aware link component used by Card and other clickable primitives. */
  linkComponent?: LinkComponent;
}

export function FluidProvider({
  children,
  shape = "rounded",
  size = "default",
  icons,
  linkComponent,
}: FluidProviderProps) {
  return (
    <MotionConfig reducedMotion="user">
      <LinkProvider component={linkComponent}>
        <ShapeProvider defaultShape={shape}>
          <SizeProvider defaultSize={size}>
            <IconProvider icons={icons}>{children}</IconProvider>
          </SizeProvider>
        </ShapeProvider>
      </LinkProvider>
    </MotionConfig>
  );
}
