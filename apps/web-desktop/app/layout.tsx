import "./style.css";
export const metadata = {
  title: "ARGOS · Intel",
  description: "Local intelligence bulletin and evidence workspace",
};
export default function Layout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
